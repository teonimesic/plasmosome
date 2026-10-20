#!/usr/bin/env python3
"""Defend the one security issue: created once, then edited, never spammed, never a person's issue.

A run rewrites only the sections for the checks it ran, so a push run keeps what the weekly run wrote.
"""

import atexit
import functools
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import sys
import tempfile


SOURCE = Path(__file__).resolve().with_name("security-issue")
ROOT = SOURCE.parent.parent
MARKER = "<!-- spec026-security-report -->"
TITLE = "Security findings on main"
RUN_URL = "https://github.com/example/repo/actions/runs/101"
LATER_URL = "https://github.com/example/repo/actions/runs/102"
LAST_URL = "https://github.com/example/repo/actions/runs/103"
COMMIT = "0123456789abcdef0123456789abcdef01234567"
WRITES = {("label", "create"), ("issue", "create"), ("issue", "edit"), ("issue", "comment")}

FAKE_GH = """\
import json, os, sys
state_path = os.environ["FAKE_GH_STATE"]
state = json.loads(open(state_path).read())
args = sys.argv[1:]
def value(flag):
    return args[args.index(flag) + 1] if flag in args else None
stdin = sys.stdin.read() if value("--body-file") == "-" else None
with open(os.environ["FAKE_GH_LOG"], "a") as log:
    log.write(json.dumps({"args": args, "stdin": stdin, "token": os.environ.get("GH_TOKEN")}) + "\\n")
command = " ".join(args[:2])
if command in state.get("fail", []):
    print(f"fake gh: {command} refused", file=sys.stderr)
    sys.exit(1)
if command in state.get("garbage", []):
    print("fake gh: not JSON")
    sys.exit(0)
if value("--repo") != "example/repo":
    print("fake gh: no --repo example/repo", file=sys.stderr)
    sys.exit(1)
def find(number):
    return next(issue for issue in state["issues"] if issue["number"] == int(number))
if command == "issue list":
    if value("--label") != "security" or value("--state") != "open":
        sys.exit("fake gh: issue list without --label security --state open")
    fields = value("--json").split(",")
    rows = [{field: issue[field] for field in fields} for issue in state["issues"]
            if issue["state"] == "open" and "security" in [name.lower() for name in issue["labels"]]]
    print(json.dumps(rows[:int(value("--limit"))]))
elif command == "label list":
    print(json.dumps([{"name": name} for name in state["labels"]]))
elif command == "label create":
    if args[2].lower() in [name.lower() for name in state["labels"]]:
        sys.exit("fake gh: label already exists")
    state["labels"].append(args[2])
elif command == "issue create":
    if value("--label") not in state["labels"]:
        sys.exit("fake gh: could not add label: not found")
    number = max([issue["number"] for issue in state["issues"]] + [0]) + 1
    state["issues"].append({"number": number, "title": value("--title"), "body": stdin,
                            "labels": [value("--label")], "state": "open", "comments": []})
    print(f"https://github.com/example/repo/issues/{number}")
elif command == "issue edit":
    find(args[2])["body"] = stdin
elif command == "issue comment":
    find(args[2])["comments"].append(stdin)
else:
    sys.exit(f"fake gh: unexpected call {args}")
open(state_path, "w").write(json.dumps(state))
"""


VULNERABILITY = {
    "kind": "vulnerability", "check": "advisories", "lockfile": "Cargo.lock", "severity": "error",
    "package": "smallvec 1.6.0", "advisory": "RUSTSEC-2021-0003",
    "title": "Buffer overflow in SmallVec::insert_many", "url": "https://github.com/servo/rust-smallvec/issues/252",
    "section": "lockfiles",
}
EXPIRED = {
    "kind": "expired-ignore", "check": None, "lockfile": None, "severity": "warning", "package": None,
    "advisory": "RUSTSEC-2000-0001", "title": "deny.toml ignores RUSTSEC-2000-0001 until 2000-01-01, which has passed",
    "url": "https://example.invalid/a", "section": "weekly",
}
TOOL_FAILURE = {
    "kind": "tool-failure", "check": "advisories", "lockfile": "fuzz/Cargo.lock", "severity": "error",
    "package": None, "advisory": None, "title": "error log: failed to fetch advisory database", "url": None,
    "section": "lockfiles",
}
LICENCE = {
    "kind": "license-exception-not-encountered", "check": "licenses", "lockfile": "fuzz/Cargo.lock",
    "severity": "warning", "package": None, "advisory": None, "title": "license exception was not encountered",
    "url": None, "section": "lockfiles",
}
CRASH = {
    "kind": "fuzz-crash", "check": None, "lockfile": None, "severity": "error", "package": "plasmosome-core",
    "advisory": None, "title": "the journal parser target crashed on input 7f3a", "url": None, "section": "fuzz",
}


def document(*findings, sections=("lockfiles", "weekly"), commit=COMMIT):
    return {"schema": 2, "event": "schedule" if "weekly" in sections else "push", "weekly": "weekly" in sections,
            "commit": commit, "date": "2026-10-09", "sections": list(sections), "findings": list(findings)}


def issue(number, body, labels=("security",), state="open"):
    return {"number": number, "title": f"Issue {number}", "body": body, "labels": list(labels), "state": state,
            "comments": []}


@functools.cache
def fake_bin():
    directory = Path(tempfile.mkdtemp(prefix="security-issue-fakes-"))
    atexit.register(shutil.rmtree, directory, True)
    (directory / "gh").write_text(f"#!{sys.executable}\n{FAKE_GH}")
    (directory / "gh").chmod(0o755)
    return directory


class Forge:
    def __init__(self, directory, issues=(), labels=(), fail=(), garbage=()):
        self.directory = Path(directory)
        self.bin = fake_bin()
        self.state = self.directory / "state.json"
        self.log = self.directory / "gh.log"
        self.findings = self.directory / "findings.json"
        self.state.write_text(json.dumps({"issues": list(issues), "labels": list(labels), "fail": list(fail),
                                          "garbage": list(garbage)}))

    def run(self, report=None, *extra, mode="write", findings=None, path=None, repo="example/repo", run_url=RUN_URL):
        if self.log.exists():
            self.log.unlink()
        if report is not None:
            self.findings.write_text(json.dumps(report))
        environment = {
            "PATH": path if path is not None else str(self.bin) + os.pathsep + os.environ.get("PATH", os.defpath),
            "HOME": str(self.directory), "GH_TOKEN": "fixture-token", "FAKE_GH_STATE": str(self.state),
            "FAKE_GH_LOG": str(self.log), "PYTHONDONTWRITEBYTECODE": "1",
        }
        argv = [sys.executable, str(SOURCE), "--findings", str(findings or self.findings), "--repo", repo,
                "--run-url", run_url, "--mode", mode, *extra]
        result = subprocess.run(argv, env=environment, capture_output=True, text=True, timeout=60)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def issues(self):
        return {item["number"]: item for item in json.loads(self.state.read_text())["issues"]}

    def labels(self):
        return json.loads(self.state.read_text())["labels"]


def writes(calls):
    return [call for call in calls if tuple(call["args"][:2]) in WRITES]


def marked(forge):
    return [number for number, item in forge.issues().items()
            if item["state"] == "open" and "security" in [name.lower() for name in item["labels"]]
            and MARKER in (item["body"] or "")]


def create_then_edit():
    with tempfile.TemporaryDirectory(prefix="security-issue-create-") as directory:
        forge = Forge(directory)
        result, calls = forge.run(document(VULNERABILITY, EXPIRED))
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert [tuple(call["args"][:2]) for call in writes(calls)] == [("label", "create"), ("issue", "create")], calls
        assert forge.labels() == ["security"], forge.labels()
        assert marked(forge) == [1], forge.issues()
        created = forge.issues()[1]
        assert created["title"] == TITLE and created["labels"] == ["security"], created
        body = created["body"]
        assert body.startswith(MARKER), body
        for text in (RUN_URL, COMMIT, "2026-10-09", "vulnerability", "Cargo.lock", "smallvec 1.6.0", "RUSTSEC-2021-0003",
                     "Buffer overflow in SmallVec::insert_many", "https://github.com/servo/rust-smallvec/issues/252",
                     "expired-ignore", "RUSTSEC-2000-0001", "https://example.invalid/a", "seven days",
                     "https://github.com/example/repo/blob/main/docs/specs/026-security-standard.md"):
            assert text in body, (text, body)
        assert all(call["token"] == "fixture-token" for call in calls), calls

        later = "fedcba9876543210fedcba9876543210fedcba98"
        result, calls = forge.run(document(VULNERABILITY, EXPIRED, commit=later), run_url=LATER_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert [(tuple(call["args"][:2]), call["args"][2]) for call in writes(calls)] == [(("issue", "edit"), "1")], calls
        assert marked(forge) == [1] and len(forge.issues()) == 1, forge.issues()
        body = forge.issues()[1]["body"]
        assert LATER_URL in body and later in body and RUN_URL not in body, body
        assert "RUSTSEC-2021-0003" in body and "RUSTSEC-2000-0001" in body, body
        assert forge.issues()[1]["comments"] == [], forge.issues()[1]

        result, calls = forge.run(document(EXPIRED), run_url=LAST_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert [tuple(call["args"][:2]) for call in writes(calls)] == [("issue", "edit"), ("issue", "comment")], calls
        body = forge.issues()[1]["body"]
        assert "RUSTSEC-2021-0003" not in body and "RUSTSEC-2000-0001" in body, body
        comments = forge.issues()[1]["comments"]
        assert len(comments) == 1 and LAST_URL in comments[0] and "no findings" in comments[0].lower(), comments
        assert "RUSTSEC-2000-0001" not in comments[0], comments

        result, calls = forge.run(document(VULNERABILITY, sections=("lockfiles",)), run_url=LATER_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        body, comments = forge.issues()[1]["body"], forge.issues()[1]["comments"]
        assert "RUSTSEC-2021-0003" in body and "RUSTSEC-2000-0001" in body and LAST_URL in body, body
        assert len(comments) == 2 and LATER_URL in comments[1], comments
        for text in ("vulnerability", "RUSTSEC-2021-0003", "smallvec 1.6.0", "Buffer overflow in SmallVec::insert_many"):
            assert text in comments[1], (text, comments[1])
        assert "RUSTSEC-2000-0001" not in comments[1], comments[1]


def a_push_run_keeps_the_weekly_findings():
    """Round 1's reproduction: the schedule run names an expired ignore, and the next push run must not drop it."""
    tests = runpy.run_path(str(SOURCE.with_name("cargo-deny-check-regression.py")))
    with tempfile.TemporaryDirectory(prefix="security-issue-push-after-schedule-") as directory:
        expired = tests["ignore"]("RUSTSEC-2000-0001", "until 2000-01-01; https://example.invalid/a; long past")
        repository = tests["Repository"](directory, {**tests["BASE"], "rust-toolchain.toml": tests["PIN"],
                                                     "deny.toml": tests["ignores"](expired)})
        same = tests["channel"](directory, "1.99.1")
        warned = {"fuzz/Cargo.toml licenses": "clean-licenses"}
        forge_directory = Path(directory) / "forge"
        forge_directory.mkdir()
        forge = Forge(forge_directory, labels=["security"])

        result, calls, summary, weekly = tests["report"](repository, "schedule", same, warned)
        assert "expired-ignore" in tests["kinds"](weekly), weekly
        result, calls = forge.run(weekly)
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert "RUSTSEC-2000-0001" in forge.issues()[1]["body"], forge.issues()[1]["body"]

        result, calls, summary, push = tests["report"](repository, "push", same, warned)
        assert tests["kinds"](push) == ["license-exception-not-encountered"], push
        result, calls = forge.run(push, run_url=LATER_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        assert "RUSTSEC-2000-0001" in body and "expired-ignore" in body, body
        assert "license-exception-not-encountered" in body and RUN_URL in body and LATER_URL in body, body
        assert [tuple(call["args"][:2]) for call in writes(calls)] == [("issue", "edit")], calls


def a_later_writer_keeps_its_own_section():
    """A writer such as the fuzz job names its own section, and the report's runs leave that section alone."""
    with tempfile.TemporaryDirectory(prefix="security-issue-later-writer-") as directory:
        forge = Forge(directory, labels=["security"])
        result, calls = forge.run(document(VULNERABILITY, EXPIRED))
        assert result.returncode == 0, (result.stdout, result.stderr)

        result, calls = forge.run(document(CRASH, sections=("fuzz",)), run_url=LATER_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        body, comments = forge.issues()[1]["body"], forge.issues()[1]["comments"]
        for text in ("RUSTSEC-2021-0003", "RUSTSEC-2000-0001", "fuzz-crash", "input 7f3a", RUN_URL, LATER_URL):
            assert text in body, (text, body)
        assert len(comments) == 1 and "input 7f3a" in comments[0] and "RUSTSEC" not in comments[0], comments

        result, calls = forge.run(document(LICENCE, sections=("lockfiles",)), run_url=LAST_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        assert "input 7f3a" in body and LATER_URL in body and "RUSTSEC-2000-0001" in body, body
        assert "RUSTSEC-2021-0003" not in body and "license-exception-not-encountered" in body, body

        changed = {**CRASH, "title": "the journal parser target crashed on input 9c1d"}
        result, calls = forge.run(document(changed, sections=("fuzz",)), run_url=LATER_URL)
        body, comments = forge.issues()[1]["body"], forge.issues()[1]["comments"]
        assert "input 9c1d" in body and "input 7f3a" not in body and LAST_URL in body, body
        assert "input 9c1d" in comments[-1] and "RUSTSEC" not in comments[-1], comments


def nothing_to_report():
    with tempfile.TemporaryDirectory(prefix="security-issue-quiet-") as directory:
        forge = Forge(directory)
        result, calls = forge.run(document())
        assert result.returncode == 0 and calls == [], (result.stdout, calls)
        assert forge.issues() == {} and forge.labels() == [], forge.issues()

    with tempfile.TemporaryDirectory(prefix="security-issue-quiet-open-") as directory:
        forge = Forge(directory, issues=[issue(4, MARKER + "\nold findings")], labels=["security"])
        before = forge.issues()
        result, calls = forge.run(document())
        assert result.returncode == 0 and calls == [], (result.stdout, calls)
        assert forge.issues() == before, forge.issues()


def a_persons_issue_is_untouched():
    person = issue(5, "I think smallvec is vulnerable.")
    with tempfile.TemporaryDirectory(prefix="security-issue-person-") as directory:
        forge = Forge(directory, issues=[person], labels=["Security"])
        result, calls = forge.run(document(VULNERABILITY))
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert [tuple(call["args"][:2]) for call in writes(calls)] == [("issue", "create")], calls
        assert forge.issues()[5] == person and marked(forge) == [6], forge.issues()
        assert forge.labels() == ["Security"] and forge.issues()[6]["labels"] == ["Security"], forge.issues()[6]

    with tempfile.TemporaryDirectory(prefix="security-issue-person-marked-") as directory:
        forge = Forge(directory, issues=[person, issue(7, MARKER + "\nold findings")], labels=["security"])
        result, calls = forge.run(document(VULNERABILITY))
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert {call["args"][2] for call in writes(calls)} == {"7"}, calls
        assert forge.issues()[5] == person, forge.issues()[5]


def duplicates():
    unlabelled = issue(2, MARKER + "\nnot labelled", labels=())
    closed = issue(1, MARKER + "\nclosed", state="closed")
    with tempfile.TemporaryDirectory(prefix="security-issue-duplicates-") as directory:
        forge = Forge(directory, labels=["security"], issues=[
            closed, unlabelled, issue(8, MARKER + "\nlater"), issue(3, MARKER + "\nearlier"), issue(11, MARKER + "\nlatest")])
        result, calls = forge.run(document(VULNERABILITY))
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert [(tuple(call["args"][:2]), call["args"][2]) for call in writes(calls)] == [
            (("issue", "edit"), "3"), (("issue", "comment"), "3")], calls
        body = forge.issues()[3]["body"]
        assert "duplicate-issue" in body and "#8" in body and "#11" in body, body
        assert "#1," not in body and "#2" not in body, body
        assert forge.issues()[8]["body"] == MARKER + "\nlater" and forge.issues()[11]["comments"] == [], forge.issues()
        assert forge.issues()[1] == closed and forge.issues()[2] == unlabelled, forge.issues()


def tool_failures():
    with tempfile.TemporaryDirectory(prefix="security-issue-failure-") as directory:
        forge = Forge(directory, labels=["security"])
        result, calls = forge.run(document(VULNERABILITY, TOOL_FAILURE))
        assert result.returncode == 1, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        assert "tool-failure" in body and "failed to fetch advisory database" in body and "fuzz/Cargo.lock" in body, body
        assert "tool failure" in result.stdout + result.stderr, result

        missing = Path(directory) / "never-written.json"
        result, calls = forge.run(findings=missing, run_url=LATER_URL)
        assert result.returncode == 1, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        assert "tool-failure" in body and "never-written.json" in body and LATER_URL in body, body
        assert "RUSTSEC-2021-0003" in body and "failed to fetch advisory database" in body, body

        other = {**VULNERABILITY, "advisory": "RUSTSEC-2099-0099"}
        unsectioned = {key: value for key, value in other.items() if key != "section"}
        for text in ("not json", "[]", json.dumps({"findings": [{"title": "no kind"}]}), json.dumps({"findings": "x"}),
                     json.dumps({**document(other), "schema": 1}),
                     json.dumps({key: value for key, value in document(other).items() if key != "sections"}),
                     json.dumps({**document(other), "sections": "lockfiles"}),
                     json.dumps(document(other, sections=("lockfiles", "lockfiles"))),
                     json.dumps(document(unsectioned)),
                     json.dumps(document({**other, "section": "weekly"}, sections=("lockfiles",))),
                     json.dumps(document({**other, "section": "fuzz -->"}, sections=("fuzz -->",))),
                     json.dumps(document({**other, "section": "Fuzz"}, sections=("Fuzz",))),
                     json.dumps(document({**other, "section": "security-issue"}, sections=("security-issue",)))):
            forge.findings.write_text(text)
            result, calls = forge.run()
            assert result.returncode == 1, (text, result.stdout, result.stderr)
            body = forge.issues()[1]["body"]
            assert "tool-failure" in body and "RUSTSEC-2021-0003" in body, (text, body)
            assert "RUSTSEC-2099-0099" not in body, (text, body)
        assert marked(forge) == [1], forge.issues()

        result, calls = forge.run(document(VULNERABILITY))
        assert result.returncode == 0, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        assert "tool-failure" not in body and "never-written.json" not in body, body

    with tempfile.TemporaryDirectory(prefix="security-issue-gh-failure-") as directory:
        for fail, expected in ((["issue list"], []), (["issue create"], [("label", "create"), ("issue", "create")]),
                               (["label create"], [("label", "create")]), (["label list"], [])):
            path = Path(directory) / fail[0].replace(" ", "-")
            path.mkdir()
            forge = Forge(path, fail=fail)
            result, calls = forge.run(document(VULNERABILITY))
            assert result.returncode == 2, (fail, result.stdout, result.stderr)
            assert f"fake gh: {fail[0]} refused" in result.stdout + result.stderr, result
            assert [tuple(call["args"][:2]) for call in writes(calls)] == expected, (fail, calls)


def gh_answers_that_cannot_be_trusted():
    crowded = [issue(number, "a person's report") for number in range(1, 1001)]
    with tempfile.TemporaryDirectory(prefix="security-issue-gh-answers-") as directory:
        for name, options, message in (
            ("crowded", {"issues": crowded, "labels": ["security"]}, "1000 or more open security issues"),
            ("issue-list-garbage", {"garbage": ["issue list"]}, "gh issue list returned invalid JSON"),
            ("label-list-garbage", {"garbage": ["label list"]}, "gh label list returned invalid JSON"),
        ):
            path = Path(directory) / name
            path.mkdir()
            forge = Forge(path, **options)
            result, calls = forge.run(document(VULNERABILITY))
            assert result.returncode == 2 and message in result.stderr, (name, result.stdout, result.stderr)
            assert writes(calls) == [], (name, calls)

        empty = Path(directory) / "empty-bin"
        empty.mkdir()
        forge = Forge(Path(directory) / "crowded")
        result, calls = forge.run(document(VULNERABILITY), path=str(empty))
        assert result.returncode == 2 and "gh issue list could not run" in result.stderr, (result.stdout, result.stderr)

        result, calls = forge.run(document(VULNERABILITY), repo="https://github.com/example/repo")
        assert result.returncode == 2 and "--repo must be OWNER/REPO" in result.stderr and calls == [], result.stderr


def dry_run():
    with tempfile.TemporaryDirectory(prefix="security-issue-dry-run-") as directory:
        forge = Forge(directory)
        result, calls = forge.run(document(VULNERABILITY, TOOL_FAILURE), mode="dry-run")
        assert result.returncode == 1, (result.stdout, result.stderr)
        assert writes(calls) == [] and forge.issues() == {} and forge.labels() == [], calls
        assert "gh label create security" in result.stdout and "gh issue create" in result.stdout, result.stdout
        assert MARKER in result.stdout and "RUSTSEC-2021-0003" in result.stdout, result.stdout
        assert "None" not in result.stdout.splitlines(), result.stdout
        assert "2 findings would be in the security issue; this was a dry run" in result.stdout, result.stdout

        path = Path(directory) / "marked"
        path.mkdir()
        forge = Forge(path, issues=[issue(4, MARKER)], labels=["security"])
        result, calls = forge.run(document(VULNERABILITY), mode="dry-run")
        assert result.returncode == 0 and writes(calls) == [], (result.stdout, calls)
        assert "gh issue edit 4" in result.stdout and "gh issue comment 4" in result.stdout, result.stdout
        assert forge.issues()[4]["body"] == MARKER, forge.issues()


def long_body():
    many = [{**VULNERABILITY, "package": f"crate{index} 1.0.0", "title": "x" * 400} for index in range(400)]
    with tempfile.TemporaryDirectory(prefix="security-issue-long-") as directory:
        forge = Forge(directory, labels=["security"])
        result, calls = forge.run(document(*many, EXPIRED))
        assert result.returncode == 0, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        assert len(body) <= 65536 and body.startswith(MARKER) and "seven days" in body, len(body)
        assert "crate0 1.0.0" in body and "crate399 1.0.0" not in body and "more findings" in body, body[-400:]
        assert "RUSTSEC-2000-0001" in body, body[-2000:]

        again = [{**item, "title": "z" * 400} for item in many]
        result, calls = forge.run(document(*again, sections=("lockfiles",)), run_url=LATER_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        body, comments = forge.issues()[1]["body"], forge.issues()[1]["comments"]
        assert len(body) <= 65536 and "RUSTSEC-2000-0001" in body and "z" * 400 in body, len(body)
        assert len(comments) == 1 and len(comments[0]) <= 65536 and "more findings" in comments[0], len(comments[0])

    with tempfile.TemporaryDirectory(prefix="security-issue-long-title-") as directory:
        forge = Forge(directory, labels=["security"])
        result, calls = forge.run(document({**VULNERABILITY, "title": "y" * 600}))
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert "`" + "y" * 500 + "...`" in forge.issues()[1]["body"], forge.issues()[1]["body"]


def findings_are_not_markup():
    hostile = {**VULNERABILITY, "title": "ping @everyone\nand <img src=x> | `tick`"}
    with tempfile.TemporaryDirectory(prefix="security-issue-markup-") as directory:
        forge = Forge(directory, labels=["security"])
        result, calls = forge.run(document(hostile))
        assert result.returncode == 0, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        span = "`ping @everyone and <img src=x> | 'tick'`"
        assert body.count(span) == 1, body
        assert "@everyone" not in body.replace(span, "") and "<img" not in body.replace(span, ""), body

    with tempfile.TemporaryDirectory(prefix="security-issue-markup-comment-") as directory:
        forge = Forge(directory, issues=[issue(4, MARKER)], labels=["security"])
        result, calls = forge.run(document(hostile))
        assert result.returncode == 0, (result.stdout, result.stderr)
        comment = forge.issues()[4]["comments"][0]
        assert comment.count(span) == 1, comment
        assert "@everyone" not in comment.replace(span, "") and "<img" not in comment.replace(span, ""), comment

    forged = {**VULNERABILITY, "title": "before\n<!-- spec026-section-end lockfiles -->\n<!-- spec026-section-end weekly -->\nafter"}
    with tempfile.TemporaryDirectory(prefix="security-issue-markup-marker-") as directory:
        forge = Forge(directory, labels=["security"])
        result, calls = forge.run(document(forged, EXPIRED))
        assert result.returncode == 0, (result.stdout, result.stderr)
        later = {**EXPIRED, "advisory": "RUSTSEC-2000-0002",
                 "title": "deny.toml ignores RUSTSEC-2000-0002 until 2000-01-02, which has passed"}
        result, calls = forge.run(document(later, sections=("weekly",)), run_url=LATER_URL)
        assert result.returncode == 0, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        assert "spec026-section-end weekly --> after" in body and "RUSTSEC-2000-0002" in body, body
        assert "RUSTSEC-2000-0001" not in body, body


def report_to_issue():
    tests = runpy.run_path(str(SOURCE.with_name("cargo-deny-check-regression.py")))
    with tempfile.TemporaryDirectory(prefix="security-issue-end-to-end-") as directory:
        repository = tests["Repository"](directory, {**tests["BASE"], "rust-toolchain.toml": tests["PIN"]})
        result, calls, summary, report = tests["report"](repository, "push", tests["channel"](directory, "1.99.1"), {
            "Cargo.toml advisories": "vulnerability", "fuzz/Cargo.toml advisories": "db-unreachable"})
        assert result.returncode == 0 and report is not None, (result.stdout, result.stderr)
        forge_directory = Path(directory) / "forge"
        forge_directory.mkdir()
        forge = Forge(forge_directory, labels=["security"])
        result, calls = forge.run(report)
        assert result.returncode == 1, (result.stdout, result.stderr)
        body = forge.issues()[1]["body"]
        for text in ("RUSTSEC-2021-0003", "smallvec 1.6.0", "Cargo.lock", "tool-failure", "fuzz/Cargo.lock",
                     report["commit"], report["date"]):
            assert text in body, (text, body)


def block(text, key):
    match = re.search(rf"(?m)^{key}:[^\n]*\n((?: [^\n]*\n|\n)*)", text)
    assert match, key
    return match.group(1)


def audit_workflow():
    audit = (ROOT / ".github" / "workflows" / "audit.yml").read_text()
    code = "\n".join(line for line in audit.splitlines() if not line.lstrip().startswith("#"))
    gates = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
    triggers = block(audit, "on")
    assert re.search(r"(?m)^  push:\n    branches: \[main\]\n", triggers), triggers
    assert re.search(r"(?m)^  schedule:\n    - cron: ", triggers), triggers
    assert re.search(r"(?m)^  workflow_dispatch:", triggers), triggers
    assert "pull_request" not in code and "paths" not in triggers, triggers
    assert re.search(r"(?m)^permissions:\n  contents: read\n(?!  )", audit), audit
    assert len(re.findall(r"(?m)^\s+issues: write$", audit)) == 1, audit
    jobs = block(audit, "jobs")
    assert re.search(r"(?m)^    permissions:\n      contents: read\n      issues: write\n", jobs), jobs
    assert re.search(r"(?m)^  cancel-in-progress: false$", block(audit, "concurrency")), audit
    assert "cargo-audit" not in code and "cargo audit" not in code, audit
    versions = re.findall(r"cargo install cargo-deny --version (\S+) --locked", audit)
    assert versions == re.findall(r"cargo install cargo-deny --version (\S+) --locked", gates) and len(versions) == 1, versions
    assert re.findall(r"(?m)^\s+key:\s*(\S+)\s*$", jobs) == [f"cargo-deny-{versions[0]}"], jobs
    for action in re.findall(r"(?m)^\s+(?:- )?uses: (\S+)", audit):
        assert re.fullmatch(r"[\w.-]+/[\w.-]+@[0-9a-f]{40}", action), action
    assert "persist-credentials: false" in audit, audit
    steps = re.split(r"(?m)^      - ", jobs)
    report = [step for step in steps if "tools/cargo-deny-check report" in step]
    issue_step = [step for step in steps if "tools/security-issue" in step]
    assert len(report) == 1 and "${{ github.event_name }}" in report[0] and '--event "$EVENT"' in report[0], report
    assert len(issue_step) == 1 and "if: ${{ !cancelled() }}" in issue_step[0], issue_step
    assert "GH_TOKEN: ${{ github.token }}" in issue_step[0], issue_step
    assert "inputs.dry-run && 'dry-run' || 'write'" in issue_step[0], issue_step
    assert steps.index(report[0]) < steps.index(issue_step[0]), steps


def main():
    audit_workflow()
    create_then_edit()
    a_push_run_keeps_the_weekly_findings()
    a_later_writer_keeps_its_own_section()
    nothing_to_report()
    a_persons_issue_is_untouched()
    duplicates()
    tool_failures()
    gh_answers_that_cannot_be_trusted()
    dry_run()
    long_body()
    findings_are_not_markup()
    report_to_issue()
    print("PASS: one marked issue created then edited, a run rewrites only its own sections, a comment only on a change, "
          "no write without findings, a person's issue untouched, duplicates reported, tool failures written and red, "
          "dry run writes nothing, audit.yml shape")


if __name__ == "__main__":
    main()
