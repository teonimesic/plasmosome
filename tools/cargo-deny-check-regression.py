#!/usr/bin/env python3
"""Defend which lockfiles cargo-deny-check reads, when it runs, how it classifies cargo-deny, and its report."""

import atexit
import contextlib
import functools
from datetime import date, datetime, timezone
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import runpy
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import threading


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
check = sys.argv[sys.argv.index("check") + 1]
names = plan.get(relative + " " + check, plan.get(relative))
fixtures = json.loads(open(os.environ["FAKE_FIXTURES"]).read())
status = 0
for name in [names] if isinstance(names, str) else names:
    for line in fixtures[name]["stderr"]:
        print(line, file=sys.stderr)
    status = status or fixtures[name]["exit"]
sys.exit(status)
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


@functools.cache
def fake_bin():
    directory = Path(tempfile.mkdtemp(prefix="cargo-deny-check-fakes-"))
    atexit.register(shutil.rmtree, directory, True)
    for name, text in (("cargo-deny", FAKE_CARGO_DENY), ("cargo", FAKE_CARGO)):
        write(directory / name, f"#!{sys.executable}\n{text}")
        (directory / name).chmod(0o755)
    return directory


class Repository:
    def __init__(self, directory, files):
        self.root = Path(directory) / "repository"
        self.bin = fake_bin()
        self.log = Path(directory) / "cargo-deny.log"
        self.cargo_log = Path(directory) / "cargo.log"
        self.plan = Path(directory) / "plan.json"
        self.summary = Path(directory) / "summary.md"
        self.root.mkdir()
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


PIN = '[toolchain]\nchannel = "1.99.1"\ncomponents = ["clippy", "rustfmt"]\nprofile = "minimal"\n'
QUIET = {f"{manifest} {check}": "licenses-quiet" if check == "licenses" else f"clean-{check}"
         for manifest in ("Cargo.toml", "fuzz/Cargo.toml") for check in ("sources", "advisories", "licenses")}
WEEKLY_KINDS = {"expired-ignore", "unparsable-ignore", "newer-stable"}


def channel(directory, version):
    path = Path(directory) / f"channel-rust-stable-{version}.toml"
    path.write_text(f'manifest-version = "2"\n\n[pkg.rust]\nversion = "{version} (0123456789 2026-10-30)"\n')
    return path.as_uri()


def ignores(*entries):
    return "[advisories]\nignore = [\n" + "".join(f"  {entry},\n" for entry in entries) + "]\n"


def ignore(advisory, reason):
    return f'{{ id = "{advisory}", reason = "{reason}" }}'


def report(repository, event, stable, overrides=None, path=None):
    output = repository.root.parent / "findings.json"
    if output.exists():
        output.unlink()
    result, calls, summary = repository.run("report", "--json-out", str(output), "--event", event, "--stable-url", stable,
                                            plan={**QUIET, **(overrides or {})}, path=path)
    document = json.loads(output.read_text()) if output.exists() else None
    return result, calls, summary, document


def kinds(document):
    return [finding["kind"] for finding in document["findings"]]


def report_covers_every_lockfile_with_warnings():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-report-") as directory:
        repository = Repository(directory, {**BASE, "rust-toolchain.toml": PIN})
        same = channel(directory, "1.99.1")
        head = git(repository.root, "rev-parse", "HEAD").strip()
        today = datetime.now(timezone.utc).date().isoformat()

        result, calls, summary, document = report(repository, "push", same)
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert document["findings"] == [], document
        assert sorted((call["manifest"], call["argv"][call["argv"].index("check") + 1]) for call in calls) == sorted(
            (manifest, check) for manifest in ("Cargo.toml", "fuzz/Cargo.toml") for check in API["CHECKS"]), calls
        assert document["event"] == "push" and document["weekly"] is False, document
        assert document["schema"] == 2 and document["sections"] == ["lockfiles"], document
        assert document["commit"] == head and document["date"] == today, document

        result, calls, summary, document = report(repository, "push", same, {
            "Cargo.toml advisories": ["notice-warned", "yanked-warned"],
            "Cargo.toml licenses": "licence-rejected",
            "fuzz/Cargo.toml advisories": "unmaintained-warned",
            "fuzz/Cargo.toml licenses": "clean-licenses",
        })
        assert result.returncode == 0, (result.stdout, result.stderr)
        found = {(item["kind"], item["lockfile"], item["package"], item["advisory"]) for item in document["findings"]}
        for expected in (
            ("notice", "Cargo.lock", "inconceivable 0.9.0", "RUSTSEC-2022-0058"),
            ("yanked", "Cargo.lock", "smallvec 1.13.0", None),
            ("rejected", "Cargo.lock", "m 0.1.0", None),
            ("unmaintained", "fuzz/Cargo.lock", "bitmaps 3.1.0", "RUSTSEC-2026-0247"),
            ("license-exception-not-encountered", "fuzz/Cargo.lock", None, None),
        ):
            assert expected in found, (expected, found)
        assert "tool-failure" not in kinds(document), document
        assert {item["section"] for item in document["findings"]} == {"lockfiles"}, document
        notice = next(item for item in document["findings"] if item["kind"] == "notice")
        assert notice["check"] == "advisories" and notice["severity"] == "warning", notice
        assert notice["url"] == "https://crates.io/crates/inconceivable" and notice["title"], notice
        for text in (result.stdout, summary):
            assert "RUSTSEC-2022-0058" in text and "bitmaps 3.1.0" in text and "fuzz/Cargo.lock" in text, text

        result, calls, summary, document = report(repository, "push", same, {
            "fuzz/Cargo.toml advisories": "vulnerability", "Cargo.toml sources": "stale-lockfile",
            "Cargo.toml advisories": "vulnerability-ignored-info", "Cargo.toml licenses": ["warn-log", "licenses-quiet"]})
        assert result.returncode == 0, (result.stdout, result.stderr)
        vulnerability = next(item for item in document["findings"] if item["kind"] == "vulnerability")
        assert vulnerability == {
            "kind": "vulnerability", "check": "advisories", "lockfile": "fuzz/Cargo.lock", "severity": "error",
            "package": "smallvec 1.6.0", "advisory": "RUSTSEC-2021-0003",
            "title": "Buffer overflow in SmallVec::insert_many", "url": "https://github.com/servo/rust-smallvec/issues/252",
            "section": "lockfiles",
        }, vulnerability
        stale = next(item for item in document["findings"] if item["kind"] == "stale-lockfile")
        assert stale["lockfile"] == "Cargo.lock" and "--locked was passed to prevent this" in stale["title"], stale
        assert "advisory-ignored" not in kinds(document), document
        assert [item["lockfile"] for item in document["findings"] if item["kind"] == "vulnerability"] == [
            "fuzz/Cargo.lock"], document
        assert not any("directory entry" in item["title"] for item in document["findings"]), document
        notes = [line for line in result.stdout.splitlines() if line.startswith("note: ")]
        assert any(line.startswith("note: Cargo.lock licenses: ") and
                   "warning log: failed to read directory entry: an invented reason" in line for line in notes), notes
        assert any(line.startswith("note: Cargo.lock advisories: ") and "advisory-ignored" in line for line in notes), notes
        assert "failed to read directory entry: an invented reason" in summary, summary


def report_tool_failures():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-report-failures-") as directory:
        repository = Repository(directory, {**BASE, "rust-toolchain.toml": PIN})
        same = channel(directory, "1.99.1")

        result, calls, summary, document = report(repository, "push", same, {
            "fuzz/Cargo.toml advisories": "db-unreachable", "Cargo.toml advisories": "vulnerability"})
        assert result.returncode == 0 and document is not None, (result.stdout, result.stderr)
        failure = next(item for item in document["findings"] if item["kind"] == "tool-failure")
        assert failure["lockfile"] == "fuzz/Cargo.lock" and failure["check"] == "advisories", failure
        assert "failed to fetch advisory database" in failure["title"], failure
        assert "vulnerability" in kinds(document), document

        result, calls, summary, document = report(repository, "push", same, {"Cargo.toml advisories": "malformed-graphs",
                                                                           "fuzz/Cargo.toml advisories": "notice-warned"})
        assert result.returncode == 0 and document is not None, (result.stdout, result.stderr)
        crashed = [item for item in document["findings"] if item["kind"] == "tool-failure"]
        assert len(crashed) == 1 and crashed[0]["lockfile"] == "Cargo.lock" and "TypeError" in crashed[0]["title"], crashed
        assert "notice" in kinds(document), document

        bare = Path(directory) / "bare-bin"
        bare.mkdir()
        os.symlink(shutil.which("git"), bare / "git")
        result, calls, summary, document = report(repository, "push", same, path=str(bare))
        assert result.returncode == 0 and calls == [], (result.stdout, result.stderr)
        assert kinds(document) == ["tool-failure"] * 6, document
        assert all("not on PATH" in item["title"] for item in document["findings"]), document

    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-report-unconfigured-") as directory:
        repository = Repository(directory, {name: text for name, text in BASE.items() if name != "deny.toml"})
        result, calls, summary, document = report(repository, "push", channel(directory, "1.99.1"))
        assert result.returncode == 0 and calls == [], (result.stdout, result.stderr)
        assert kinds(document) == ["tool-failure"] and "deny.toml" in document["findings"][0]["title"], document
        assert document["sections"] == ["lockfiles"] and document["findings"][0]["section"] == "lockfiles", document


def report_weekly_checks():
    """The CLI wiring of the weekly checks; ignore_expiry and stable_comparison cover their logic."""
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-report-weekly-") as directory:
        repository = Repository(directory, {**BASE, "rust-toolchain.toml": PIN, "deny.toml": ignores(
            ignore("RUSTSEC-2000-0001", "until 2000-01-01; https://example.invalid/a; long past"),
            ignore("RUSTSEC-2000-0002", "until 2999-12-31; https://example.invalid/b; still valid"),
            ignore("RUSTSEC-2000-0003", "until 2026-9-30; https://example.invalid/c; not zero-padded, naïve"),
        )})
        newer = channel(directory, "1.100.0")
        missing = (Path(directory) / "no-such-channel.toml").as_uri()

        result, calls, summary, document = report(repository, "schedule", newer, {"Cargo.toml advisories": "notice-warned"})
        assert result.returncode == 0 and document["weekly"] is True, (result.stdout, result.stderr)
        assert document["sections"] == ["lockfiles", "weekly"], document
        found = {(item["kind"], item["advisory"], item["section"]) for item in document["findings"]}
        assert found == {("notice", "RUSTSEC-2022-0058", "lockfiles"), ("expired-ignore", "RUSTSEC-2000-0001", "weekly"),
                         ("unparsable-ignore", "RUSTSEC-2000-0003", "weekly"), ("newer-stable", None, "weekly")}, document
        stable = next(item for item in document["findings"] if item["kind"] == "newer-stable")
        assert stable["package"] == "rust 1.100.0" and "1.99.1" in stable["title"] and stable["url"], stable
        assert "naïve" in next(item["title"] for item in document["findings"] if item["kind"] == "unparsable-ignore")
        assert "RUSTSEC-2000-0001" in result.stdout and "RUSTSEC-2000-0001" in summary, (result.stdout, summary)

        result, calls, summary, document = report(repository, "workflow_dispatch", missing)
        assert result.returncode == 0 and document["weekly"] is True, (result.stdout, result.stderr)
        assert document["sections"] == ["lockfiles", "weekly"], document
        found = {(item["kind"], item["advisory"], item["section"]) for item in document["findings"]}
        assert found == {("expired-ignore", "RUSTSEC-2000-0001", "weekly"), ("unparsable-ignore", "RUSTSEC-2000-0003", "weekly"),
                         ("tool-failure", None, "weekly")}, document
        failure = next(item for item in document["findings"] if item["kind"] == "tool-failure")
        assert "no-such-channel.toml" in failure["title"], failure

        result, calls, summary, document = report(repository, "push", missing)
        assert result.returncode == 0 and document["weekly"] is False, (result.stdout, result.stderr)
        assert document["sections"] == ["lockfiles"] and document["findings"] == [], document


def ignore_expiry():
    today = date(2026, 10, 9)
    text = ignores(
        ignore("RUSTSEC-2026-0001", "until 2026-10-09; https://example.invalid/a; dated today"),
        ignore("RUSTSEC-2026-0002", "until 2026-10-08; https://example.invalid/b; dated yesterday"),
        ignore("RUSTSEC-2026-0003", "until 2026-9-30; https://example.invalid/c; not zero-padded"),
        ignore("RUSTSEC-2026-0004", "until 2026-12-01; no link; a reason without a link"),
        ignore("RUSTSEC-2026-0005", "until 2026-02-30; https://example.invalid/e; no such day"),
        ignore("RUSTSEC-2026-0006", "until 2026-12-01; https://example.invalid/f; "),
        ignore("RUSTSEC-2026-0007", "until 2026-12-01; https://example.invalid/g; why\\nwith a second line"),
        ignore("RUSTSEC-2026-0008", "until \u0662\u0660\u0662\u0666-12-01; https://example.invalid/h; other digits"),
        '"RUSTSEC-2026-0009"',
        '{ id = "RUSTSEC-2026-0010" }',
        '{ crate = "smallvec@1.13.0", reason = "until 2026-10-01; https://example.invalid/k; a yanked crate" }',
    )
    found = API["ignore_findings"](text, today)
    assert {(item["kind"], item["advisory"]) for item in found} == {
        ("expired-ignore", "RUSTSEC-2026-0002"),
        *(("unparsable-ignore", f"RUSTSEC-2026-{number:04d}") for number in range(3, 11)),
        ("expired-ignore", "smallvec@1.13.0"),
    }, found
    expired = next(item for item in found if item["advisory"] == "RUSTSEC-2026-0002")
    assert "2026-10-08" in expired["title"] and expired["url"] == "https://example.invalid/b", expired
    assert API["ignore_findings"]("[sources]\n", today) == []
    assert API["ignore_findings"]("[advisories]\nignore = []\n", today) == []
    for broken in ("[advisories\n", "[advisories]\nignore = 3\n", "advisories = 3\n", "[advisories]\nignore = [3]\n"):
        found = API["ignore_findings"](broken, today)
        assert [item["kind"] for item in found] == ["tool-failure"] and "deny.toml" in found[0]["title"], (broken, found)


def serve(answer):
    """Answer one HTTP request on localhost with these bytes, then close the connection."""
    listener = socket.create_server(("127.0.0.1", 0))

    def respond():
        with listener:
            connection, _ = listener.accept()
            with connection:
                connection.recv(65536)
                connection.sendall(answer)

    thread = threading.Thread(target=respond, daemon=True)
    thread.start()
    return f"http://127.0.0.1:{listener.getsockname()[1]}/channel-rust-stable.toml", thread


def stable_comparison():
    for answer, name in ((b"garbage\r\n\r\n", "BadStatusLine"),
                         (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n40\r\nmanifest-version", "IncompleteRead")):
        url, thread = serve(answer)
        found = API["stable_findings"](PIN, url)
        thread.join(10)
        assert [item["kind"] for item in found] == ["tool-failure"] and name in found[0]["title"], (name, found)
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-stable-") as directory:
        for stable, pin, newer in (
            ("1.99.1", "1.99.1", False), ("1.99.2", "1.99.1", True), ("1.100.0", "1.99.1", True),
            ("1.98.9", "1.99.1", False), ("2.0.0", "1.99.1", True), ("1.99.1", "1.100.0", False),
        ):
            found = API["stable_findings"](PIN.replace("1.99.1", pin), channel(directory, stable))
            assert [item["kind"] for item in found] == (["newer-stable"] if newer else []), (stable, pin, found)
        for pin in ("stable", "1.99", "1.99.1-beta"):
            found = API["stable_findings"](PIN.replace("1.99.1", pin), channel(directory, "1.99.1"))
            assert [item["kind"] for item in found] == ["tool-failure"], (pin, found)
        broken = Path(directory) / "broken.toml"
        for text in ("not toml [", "[pkg.rust]\n", '[pkg.rust]\nversion = "nightly"\n'):
            broken.write_text(text)
            found = API["stable_findings"](PIN, broken.as_uri())
            assert [item["kind"] for item in found] == ["tool-failure"], (text, found)
        found = API["stable_findings"](PIN, (Path(directory) / "absent.toml").as_uri())
        assert [item["kind"] for item in found] == ["tool-failure"], found
        namespace = API["stable_findings"].__globals__
        saved = namespace["CHANNEL_LIMIT"]
        namespace["CHANNEL_LIMIT"] = 16
        try:
            found = API["stable_findings"](PIN, channel(directory, "1.100.0"))
        finally:
            namespace["CHANNEL_LIMIT"] = saved
        assert [item["kind"] for item in found] == ["tool-failure"] and "over 16 bytes" in found[0]["title"], found


def report_arguments():
    with tempfile.TemporaryDirectory(prefix="cargo-deny-check-report-arguments-") as directory:
        repository = Repository(directory, BASE)
        output = Path(directory) / "findings.json"
        for args in (("report", "--json-out", str(output)), ("report", "--event", "push"),
                     ("report", "--json-out", str(output), "--event", "push", "--changed-since", "HEAD"),
                     ("sources", "--event", "push"), ("advisories", "--json-out", str(output)),
                     ("report", "--json-out", str(output), "--event", "pull_request")):
            result, calls, summary = repository.run(*args, plan=QUIET)
            assert result.returncode == 2 and "usage:" in result.stderr, (args, result.stdout, result.stderr)
            assert calls == [] and not output.exists(), (args, calls)


def main():
    ignore_expiry()
    stable_comparison()
    report_covers_every_lockfile_with_warnings()
    report_tool_failures()
    report_weekly_checks()
    report_arguments()
    classification()
    single_rules()
    timeout()
    tracked_lockfiles_and_invocation()
    logged_command()
    trigger()
    outcomes()
    cache_key_names_installed_version()
    print("PASS: cargo-deny output classified by code, tracked lockfiles at any depth, manifest trigger, worst outcome wins, "
          "report with warnings and weekly checks")


if __name__ == "__main__":
    main()
