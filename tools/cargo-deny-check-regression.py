#!/usr/bin/env python3
"""Defend which lockfiles cargo-deny-check reads, when it runs, and how it classifies cargo-deny."""

import contextlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import runpy
import shlex
import shutil
import subprocess
import sys
import tempfile


SOURCE = Path(__file__).resolve().with_name("cargo-deny-check")
FIXTURES = json.loads(Path(__file__).resolve().with_name("cargo-deny-check-fixtures.json").read_text())
API = runpy.run_path(str(SOURCE))
SKIPPED = "skipped: this change touches no Cargo.toml or Cargo.lock"
TOOL_FAILURE = "cargo-deny tool failure (not a finding):"
REFUSAL = "cargo-deny refusal:"
IGNORE_FORM = '{ id = "RUSTSEC-2021-0003", reason = "until YYYY-MM-DD; <link>; <why>" }'
LOCK_HINT = "Cargo.lock does not match the manifests: run `cargo update --workspace` (or any cargo build) and commit Cargo.lock"
CARGO_HINT = "remove the --locked flag"

FAKE_CARGO_DENY = """\
import json, os, sys
plan = json.loads(open(os.environ["FAKE_PLAN"]).read())
manifest = sys.argv[sys.argv.index("--manifest-path") + 1]
relative = os.path.relpath(os.path.abspath(manifest), os.getcwd())
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(json.dumps({"argv": sys.argv, "manifest": relative, "color": os.environ.get("CARGO_TERM_COLOR")}) + "\\n")
fixture = json.loads(open(os.environ["FAKE_FIXTURES"]).read())[plan[relative]]
for line in fixture["stderr"]:
    print(line, file=sys.stderr)
sys.exit(fixture["exit"])
"""

FAKE_CARGO = """\
import json, os, sys
with open(os.environ["FAKE_CARGO_LOG"], "a") as log:
    log.write(json.dumps(sys.argv) + "\\n")
sys.exit(1)
"""


def classified(fixture, check):
    entry = FIXTURES[fixture]
    return API["classify"](check, entry["stderr"], entry["exit"])


def records(fixture, kind):
    return [line for line in FIXTURES[fixture]["stderr"] if json.loads(line)["type"] == kind]


def single_rules():
    for name, check, lines, returncode, fragment in (
        ("only another check's summary", "advisories", FIXTURES["clean-sources"]["stderr"], 0,
         "a summary for another check"),
        ("a refusing code of another check", "sources",
         records("vulnerability", "diagnostic") + records("clean-sources", "summary"), 1,
         "a code this wrapper does not know: error vulnerability RUSTSEC-2021-0003"),
        ("a reported code of another check", "advisories",
         records("licence-rejected", "diagnostic")[:1] + records("clean-advisories", "summary"), 4,
         "a code this wrapper does not know: error rejected"),
        ("a nonzero exit with only a warning", "advisories", FIXTURES["notice-warned"]["stderr"], 1, "exited 1"),
        ("a signal kill", "advisories", FIXTURES["clean-advisories"]["stderr"], -9, "exited -9"),
    ):
        outcome = API["classify"](check, lines, returncode)
        assert outcome["status"] == "tool failure" and outcome["refusals"] == [], (name, outcome)
        assert len(outcome["failures"]) == 1 and fragment in outcome["failures"][0], (name, outcome)

    both = API["classify"]("advisories", FIXTURES["vulnerability"]["stderr"] + FIXTURES["db-unreachable"]["stderr"], 1)
    assert both["status"] == "tool failure" and both["refusals"] and both["failures"], both


def timeout():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-timeout-") as directory:
        sleeper = Path(directory) / "cargo-deny"
        write(sleeper, f"#!{sys.executable}\nimport time\ntime.sleep(60)\n")
        sleeper.chmod(0o755)
        namespace = API["run"].__globals__
        saved = namespace["TIMEOUT"], os.environ["PATH"]
        namespace["TIMEOUT"] = 0.5
        os.environ["PATH"] = directory + os.pathsep + saved[1]
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                outcome = API["run"](Path(directory), "advisories", PurePosixPath("Cargo.lock"))
        finally:
            namespace["TIMEOUT"], os.environ["PATH"] = saved
        assert outcome["status"] == "tool failure" and outcome["code"] == 2, outcome
        assert "could not complete" in outcome["failures"][0] and "timed out" in outcome["failures"][0], outcome


def cache_key_names_installed_version():
    workflow = (SOURCE.parent.parent / ".github" / "workflows" / "ci.yml").read_text()
    versions = re.findall(r"cargo install cargo-deny --version (\S+) --locked", workflow)
    assert len(versions) == 1, versions
    job = next(block for block in re.split(r"(?m)^(?=  [\w-]+:\s*$)", workflow) if "cargo install cargo-deny" in block)
    keys = re.findall(r"(?m)^\s+key:\s*(\S+)\s*$", job)
    assert len(keys) == 1 and versions[0] in re.split(r"[^0-9A-Za-z.]+", keys[0]), (versions, keys)


def classification():
    cases = [
        ("clean-sources", "sources", "pass", []),
        ("clean-advisories", "advisories", "pass", []),
        ("clean-licenses", "licenses", "pass", ["license-exception-not-encountered", "libfuzzer-sys"]),
        ("vulnerability", "advisories", "refusal",
         ["vulnerability", "RUSTSEC-2021-0003", "smallvec 1.6.0", "Buffer overflow in SmallVec::insert_many", IGNORE_FORM]),
        ("vulnerability-ignored", "advisories", "pass", []),
        ("unsound-transitive", "advisories", "refusal", ["unsound", "RUSTSEC-2021-0145", "atty 0.2.14", "RUSTSEC-2024-0375"]),
        ("notice-warned", "advisories", "pass", ["notice", "RUSTSEC-2022-0058", "inconceivable 0.9.0"]),
        ("notice-unwarned", "advisories", "pass", ["notice", "RUSTSEC-2022-0058"]),
        ("unmaintained-warned", "advisories", "pass", ["unmaintained", "RUSTSEC-2026-0247", "bitmaps 3.1.0"]),
        ("unmaintained-unwarned", "advisories", "pass", ["unmaintained", "RUSTSEC-2026-0247"]),
        ("yanked-warned", "advisories", "pass", ["yanked", "smallvec 1.13.0"]),
        ("yanked-denied", "advisories", "pass", ["yanked", "smallvec 1.13.0"]),
        ("ignore-unknown-and-undetected", "advisories", "refusal",
         ["RUSTSEC-2021-0003", "unknown-advisory", "RUSTSEC-2099-9999", "advisory-not-detected", "RUSTSEC-2021-0145"]),
        ("ignore-bad-form", "advisories", "tool failure", ["unexpected keys", "expires"]),
        ("db-unreachable", "advisories", "tool failure", ["failed to fetch advisory database"]),
        ("stale-lockfile", "sources", "refusal", ["because --locked was passed to prevent this", LOCK_HINT]),
        ("git-source", "sources", "refusal", ["source-not-allowed", "gitdep 0.1.0", "git+file:///work/gitsrc"]),
        ("licence-rejected", "licenses", "pass", ["rejected", "m 0.1.0", "GPL-3.0-only"]),
        ("unknown-error-code", "advisories", "tool failure", ["a-code-cargo-deny-0-20-2-never-writes"]),
        ("index-failure", "advisories", "tool failure", ["index-failure"]),
        ("error-log-with-summary", "advisories", "tool failure", ["feature sets for crates may not be correct"]),
        ("not-json", "advisories", "tool failure", ["thread 'main' panicked"]),
        ("stray-line", "advisories", "tool failure", ["a line that is not JSON: Updating crates.io index"]),
        ("nonzero-without-diagnostic", "advisories", "tool failure", ["exited 1"]),
        ("silent", "advisories", "tool failure", ["no advisories summary"]),
        ("unknown-record-type", "advisories", "tool failure", ["finding"]),
        ("vulnerability", "sources", "tool failure", ["vulnerability"]),
        ("git-source", "advisories", "tool failure", ["source-not-allowed"]),
    ]
    for fixture, check, status, expected in cases:
        outcome = classified(fixture, check)
        assert outcome["status"] == status, (fixture, check, outcome)
        if status == "pass":
            assert outcome["refusals"] == [] and outcome["failures"] == [], (fixture, check, outcome)
        if status == "refusal":
            assert outcome["refusals"] and outcome["failures"] == [], (fixture, check, outcome)
        if status == "tool failure":
            assert outcome["failures"], (fixture, check, outcome)
        text = "\n".join(outcome["refusals"] + outcome["failures"] + outcome["reported"])
        for fragment in expected:
            assert fragment in text, (fixture, check, fragment, text)

    for fixture in ("notice-unwarned", "unmaintained-unwarned", "yanked-denied", "licence-rejected"):
        assert FIXTURES[fixture]["exit"] != 0, fixture
    assert classified("db-unreachable", "advisories")["refusals"] == []

    stale = classified("stale-lockfile", "sources")
    assert len(stale["refusals"]) == 1 and stale["refusals"][0].splitlines()[-1].strip() == LOCK_HINT, stale
    assert CARGO_HINT not in "\n".join(stale["refusals"] + stale["reported"]), stale
    stale_and_broken = API["classify"](
        "sources", FIXTURES["stale-lockfile"]["stderr"] + FIXTURES["db-unreachable"]["stderr"], 1)
    assert stale_and_broken["status"] == "tool failure", stale_and_broken

    malformed = API["classify"]("advisories", ["", "[]", '{"type": "log"}'], 0)
    assert malformed["status"] == "tool failure" and len(malformed["failures"]) == 2, malformed
    warning = "unable to find a config path, falling back to default config"
    logged = API["classify"]("sources", [
        json.dumps({"fields": {"level": "WARN", "message": warning}, "type": "log"}),
        json.dumps({"fields": {"level": "INFO", "message": "checking sources"}, "type": "log"}),
        FIXTURES["clean-sources"]["stderr"][0],
    ], 0)
    assert logged["status"] == "pass" and logged["reported"] == [f"warning log: {warning}"], logged


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def git(repository, *args):
    return subprocess.run(["git", *args], cwd=repository, check=True, capture_output=True, text=True).stdout


def commit(repository, message):
    git(repository, "add", "-A")
    git(repository, "-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false",
        "commit", "--quiet", "--no-verify", "-m", message)


class Repository:
    def __init__(self, directory, files):
        self.root = Path(directory) / "repository"
        self.bin = Path(directory) / "bin"
        self.log = Path(directory) / "cargo-deny.log"
        self.cargo_log = Path(directory) / "cargo.log"
        self.plan = Path(directory) / "plan.json"
        self.summary = Path(directory) / "summary.md"
        self.root.mkdir()
        self.bin.mkdir()
        for name, text in (("cargo-deny", FAKE_CARGO_DENY), ("cargo", FAKE_CARGO)):
            write(self.bin / name, f"#!{sys.executable}\n{text}")
            (self.bin / name).chmod(0o755)
        git(self.root, "init", "--quiet")
        for name, text in files.items():
            write(self.root / name, text)
        commit(self.root, "base")

    def change(self, edits, renames=()):
        for old, new in renames:
            git(self.root, "mv", old, new)
        for name, text in edits.items():
            write(self.root / name, text)
        commit(self.root, "change")

    def run(self, *args, plan=None, path=None, summary=None):
        self.plan.write_text(json.dumps(plan or {}))
        for stale in (self.log, self.summary):
            if stale.exists():
                stale.unlink()
        target = Path(summary) if summary is not None else self.summary
        environment = {
            "PATH": path if path is not None else str(self.bin) + os.pathsep + os.environ.get("PATH", os.defpath),
            "HOME": os.environ.get("HOME", str(self.root)),
            "FAKE_PLAN": str(self.plan), "FAKE_LOG": str(self.log), "FAKE_CARGO_LOG": str(self.cargo_log),
            "FAKE_FIXTURES": str(SOURCE.with_name("cargo-deny-check-fixtures.json")),
            "GITHUB_STEP_SUMMARY": str(target), "PYTHONDONTWRITEBYTECODE": "1", "CARGO_TERM_COLOR": "always",
        }
        result = subprocess.run([sys.executable, str(SOURCE), *args], cwd=self.root, env=environment,
                                capture_output=True, text=True, timeout=60)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        summary = target.read_text() if target.is_file() else ""
        assert not self.cargo_log.exists(), f"cargo was invoked: {self.cargo_log.read_text()}"
        return result, calls, summary


BASE = {
    "Cargo.toml": "[workspace]\nmembers = [\"crates/a\"]\n",
    "Cargo.lock": "version = 4\n",
    "deny.toml": "[sources]\n",
    "crates/a/Cargo.toml": "[package]\nname = \"a\"\n",
    "fuzz/Cargo.toml": "[package]\nname = \"fuzz\"\n",
    "fuzz/Cargo.lock": "version = 4\n",
    "README.md": "readme\n",
    "docs/Cargo.toml.md": "notes\n",
    "vendor/Cargo.lock.orig": "version = 4\n",
}


def plan(check, overrides=None):
    return {"Cargo.toml": f"clean-{check}", "fuzz/Cargo.toml": f"clean-{check}", **(overrides or {})}


def tracked_lockfiles_and_invocation():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-lockfiles-") as directory:
        repository = Repository(directory, BASE)
        write(repository.root / "scratch/Cargo.lock", "version = 4\n")
        write(repository.root / "scratch/Cargo.toml", "[package]\nname = \"scratch\"\n")
        for check, extra in (("sources", []), ("advisories", ["-W", "notice", "-W", "unmaintained"]), ("licenses", [])):
            result, calls, summary = repository.run(check, plan=plan(check))
            assert result.returncode == 0, (check, result.stdout, result.stderr)
            assert [call["manifest"] for call in calls] == ["Cargo.toml", "fuzz/Cargo.toml"], calls
            for call in calls:
                argv = call["argv"]
                assert Path(argv[0]).name == "cargo-deny", argv
                assert "--locked" in argv and "--all-features" in argv and "--workspace" in argv, argv
                assert argv[argv.index("--format") + 1] == "json", argv
                assert Path(argv[argv.index("--config") + 1]).resolve() == (repository.root / "deny.toml").resolve(), argv
                assert argv[-2 - len(extra):] == ["check", check, *extra], argv
                assert call["color"] == "never", call
            assert "`Cargo.lock`" in summary and "`fuzz/Cargo.lock`" in summary, summary
            assert "scratch" not in result.stdout + summary


def logged_command():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-quoting-") as directory:
        repository = Repository(directory, {
            "deny.toml": "[sources]\n", "a b/Cargo.toml": "[package]\nname = \"a\"\n", "a b/Cargo.lock": "version = 4\n"})
        result, calls, summary = repository.run("sources", plan={"a b/Cargo.toml": "clean-sources"})
        assert result.returncode == 0 and len(calls) == 1, (result.stdout, result.stderr)
        logged = [line[len("$ "):] for line in result.stdout.splitlines() if line.startswith("$ ")]
        assert len(logged) == 1 and shlex.split(logged[0])[0] == "cargo-deny", logged
        assert shlex.split(logged[0])[1:] == calls[0]["argv"][1:], (logged, calls)


def trigger():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-trigger-") as directory:
        repository = Repository(directory, BASE)
        repository.change({"README.md": "readme, edited\n", "docs/Cargo.toml.md": "notes, edited\n"})
        result, calls, summary = repository.run("advisories", "--changed-since", "HEAD^1", plan=plan("advisories"))
        assert result.returncode == 0 and calls == [], (result.stdout, calls)
        assert SKIPPED in result.stdout and SKIPPED in summary, (result.stdout, summary)

        for edits, renames in (
            ({"crates/a/Cargo.toml": "[package]\nname = \"a\"\n# comment\n"}, ()),
            ({"fuzz/Cargo.lock": "version = 4\n# changed\n"}, ()),
            ({"Cargo.lock": "version = 4\n# changed\n"}, ()),
            ({}, (("crates/a/Cargo.toml", "crates/a/Cargo.toml.bak"),)),
        ):
            repository.change(edits, renames)
            result, calls, summary = repository.run("advisories", "--changed-since", "HEAD^1",
                                                    plan=plan("advisories", {"Cargo.toml": "vulnerability"}))
            assert result.returncode == 1, (edits, renames, result.stdout, result.stderr)
            assert len(calls) == 2 and SKIPPED not in result.stdout, (edits, renames, result.stdout)
            assert REFUSAL in result.stdout and REFUSAL in summary, result.stdout

        result, calls, summary = repository.run("advisories", "--changed-since", "no-such-revision", plan=plan("advisories"))
        assert result.returncode == 2 and calls == [] and TOOL_FAILURE in result.stdout, result.stdout


def outcomes():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-outcomes-") as directory:
        repository = Repository(directory, BASE)

        result, calls, summary = repository.run("advisories", plan=plan("advisories", {"Cargo.toml": "vulnerability"}))
        assert result.returncode == 1, (result.stdout, result.stderr)
        for text in (result.stdout, summary):
            assert REFUSAL in text and "RUSTSEC-2021-0003" in text and "smallvec 1.6.0" in text, text
            assert TOOL_FAILURE not in text, text
        assert IGNORE_FORM in result.stdout, result.stdout

        result, calls, summary = repository.run("advisories", plan=plan("advisories", {"fuzz/Cargo.toml": "db-unreachable"}))
        assert result.returncode == 2, result.stdout
        assert TOOL_FAILURE in result.stdout and REFUSAL not in result.stdout, result.stdout
        assert TOOL_FAILURE in summary, summary

        result, calls, summary = repository.run(
            "advisories", plan=plan("advisories", {"Cargo.toml": "vulnerability", "fuzz/Cargo.toml": "db-unreachable"}))
        assert result.returncode == 2 and len(calls) == 2, result.stdout
        assert REFUSAL in result.stdout and TOOL_FAILURE in result.stdout, result.stdout

        result, calls, summary = repository.run("advisories", plan=plan("advisories", {"Cargo.toml": "notice-unwarned"}))
        assert result.returncode == 0 and "RUSTSEC-2022-0058" in summary, (result.stdout, summary)

        result, calls, summary = repository.run("sources", plan=plan("sources", {"fuzz/Cargo.toml": "stale-lockfile"}))
        assert result.returncode == 1 and "because --locked was passed to prevent this" in result.stdout, result.stdout
        assert result.stdout.splitlines()[-1].strip() == LOCK_HINT and CARGO_HINT not in result.stdout, result.stdout
        assert REFUSAL in summary and LOCK_HINT in summary and TOOL_FAILURE not in result.stdout + summary, summary

        result, calls, summary = repository.run("advisories", plan=plan("advisories", {"Cargo.toml": "malformed-graphs"}))
        assert result.returncode == 2 and TOOL_FAILURE + " the wrapper crashed" in result.stdout, result.stdout
        assert "Traceback" in result.stdout and "TypeError" in result.stdout, result.stdout

        directory_summary = Path(directory) / "summary-directory"
        directory_summary.mkdir()
        result, calls, summary = repository.run("sources", plan=plan("sources"), summary=directory_summary)
        assert result.returncode == 2 and TOOL_FAILURE + " the wrapper crashed" in result.stdout, result.stdout
        assert "IsADirectoryError" in result.stdout, result.stdout

        result, calls, summary = repository.run("sources", plan=plan("sources", {"fuzz/Cargo.toml": "git-source"}))
        assert result.returncode == 1 and "gitdep 0.1.0" in result.stdout, result.stdout

        result, calls, summary = repository.run("licenses", plan=plan("licenses", {"Cargo.toml": "licence-rejected"}))
        assert result.returncode == 0, result.stdout
        assert "GPL-3.0-only" in summary and "m 0.1.0" in summary, summary

        bare = Path(directory) / "bare-bin"
        bare.mkdir()
        os.symlink(shutil.which("git"), bare / "git")
        result, calls, summary = repository.run("sources", plan=plan("sources"), path=str(bare))
        assert result.returncode == 2 and calls == [], result.stdout
        assert TOOL_FAILURE in result.stdout and "cargo-deny" in result.stdout, result.stdout

        write(bare / "cargo-deny", "not a program\n")
        result, calls, summary = repository.run("sources", plan=plan("sources"), path=str(bare))
        assert result.returncode == 2 and calls == [] and "could not complete" in result.stdout, result.stdout

        empty = Path(directory) / "empty-bin"
        empty.mkdir()
        result, calls, summary = repository.run("sources", plan=plan("sources"), path=str(empty))
        assert result.returncode == 2 and calls == [] and TOOL_FAILURE in result.stdout, result.stdout
        assert "git rev-parse could not run" in result.stdout, result.stdout

    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-nothing-") as directory:
        repository = Repository(directory, {"README.md": "readme\n", "deny.toml": "[sources]\n"})
        result, calls, summary = repository.run("sources")
        assert result.returncode == 2 and calls == [] and TOOL_FAILURE in result.stdout, result.stdout
        assert "Cargo.lock" in result.stdout, result.stdout

    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-unconfigured-") as directory:
        repository = Repository(directory, {name: text for name, text in BASE.items() if name != "deny.toml"})
        result, calls, summary = repository.run("sources", plan=plan("sources"))
        assert result.returncode == 2 and calls == [] and TOOL_FAILURE in result.stdout, result.stdout
        assert "deny.toml" in result.stdout, result.stdout


def main():
    classification()
    single_rules()
    timeout()
    tracked_lockfiles_and_invocation()
    logged_command()
    trigger()
    outcomes()
    cache_key_names_installed_version()
    print("PASS: cargo-deny output classified by code, tracked lockfiles at any depth, manifest trigger, worst outcome wins")


if __name__ == "__main__":
    main()
