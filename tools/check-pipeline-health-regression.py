#!/usr/bin/env python3
"""Defend health-report uncertainty, delivery windows, pagination, security issues and read-only failure boundaries."""

from contextlib import contextmanager, redirect_stderr, redirect_stdout
from datetime import datetime, timedelta, timezone
import errno
import io
import json
import os
from pathlib import Path
import runpy
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import traceback


SOURCE = Path(__file__).resolve().with_name("check-pipeline-health")
API = runpy.run_path(str(SOURCE))
NOW = datetime(2026, 9, 12, 15, tzinfo=timezone.utc)
START = NOW - timedelta(hours=1)


class Skipped(Exception):
    """A row that cannot run on this platform, with the reason the run prints."""


def only_on_macos(reason):
    if sys.platform != "darwin":
        raise Skipped(f"macOS only: {reason}")


def responses(*items):
    pending = iter(items)

    def run(argv):
        value = next(pending)
        if isinstance(value, Exception):
            raise value
        return value

    return run


def native_rows(before, after=None, *, first_commit="a", last_commit="a"):
    return API["collect_native"](responses(
        {"commit": first_commit, "branch": "main"}, before,
        before if after is None else after, {"commit": last_commit, "branch": "main"},
    ))


def pull(number, state="MERGED", when=NOW):
    return {
        "number": number, "url": f"https://github.com/example/repo/pull/{number}",
        "title": f"PR {number}", "state": state, "isDraft": False, "headRefOid": f"{number:040x}",
        "createdAt": (START - timedelta(days=2)).isoformat(), "updatedAt": NOW.isoformat(),
        "closedAt": when.isoformat() if state != "OPEN" else None,
        "mergedAt": when.isoformat() if state == "MERGED" else None,
        "mergeCommit": {"oid": f"{number + 100:040x}"} if state == "MERGED" else None,
        "additions": 17, "deletions": 5,
    }


def page(nodes, total, cursor=None):
    return {"data": {"repository": {"pullRequests": {
        "nodes": nodes, "totalCount": total,
        "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
    }}}}


def github(*pages, start=START, end=NOW):
    return API["collect_github"](responses(*pages), repository="example/repo", window_start=start, observed_at=end)[0]


def native_uncertainty():
    rows = [
        {"id": "planning", "title": "Plan", "status": "planning", "assignee": "author"},
        {"id": "review", "title": "Review", "status": "review"},
        {"id": "blocked", "title": "Wait", "status": "blocked", "assignee": "original-owner"},
        {"id": "done", "title": "Done", "status": "closed"},
    ]
    report = native_rows(rows)
    assert report["collection_status"] == "complete"
    assert report["wip"]["count"] == 2 and report["owned_blocked"]["count"] == 1
    assert report["owned_blocked"]["ids"] == ["blocked"]
    assert report["counts_by_status"]["closed"] == 1
    unassigned = next(task for task in report["tasks"] if task["id"] == "review")
    assert unassigned["assignment_state"] == "unassigned"
    literal_owner = native_rows([{**rows[2], "assignee": "\t"}])
    assert literal_owner["tasks"][0]["assignment_state"] == "owned"
    assert literal_owner["owned_blocked"]["count"] == 1

    missing = native_rows([{**rows[0], "assignee": None}, {"id": "bad", "title": "Unavailable status"},
                           {"id": "custom", "title": "Future phase", "status": "unexpected"}])
    assert missing["collection_status"] == "partial"
    assert missing["tasks"][0]["assignment_state"] == "unavailable"
    assert missing["unavailable_status_count"] == 1 and missing["counts_by_status"]["unexpected"] is None
    assert missing["observed_row_counts_by_status"]["unexpected"] == 1
    assert missing["wip"]["count"] is None and missing["wip"]["observed_count"] == 1
    assert {task["id"] for task in missing["tasks"]} == {"planning", "bad", "custom"}

    duplicate = native_rows([rows[0], rows[0]])
    assert duplicate["total_tasks"] is None and duplicate["counts_by_status"]["planning"] is None
    assert duplicate["wip"]["count"] is None and duplicate["wip"]["observed_count"] == 1
    assert duplicate["wip"]["by_status"]["planning"]["ids"] == ["planning"]
    unknown_identity = native_rows([{"title": "No identity", "status": "planning"}])
    assert unknown_identity["wip"]["count"] is None and unknown_identity["total_tasks"] is None
    unknown_status = native_rows([{"id": "unclassified", "title": "No status"}])
    assert unknown_status["wip"]["count"] is None and unknown_status["owned_blocked"]["count"] is None
    assert unknown_status["counts_by_status"] is None
    unknown_blocked_owner = native_rows([{**rows[2], "assignee": None}])
    assert unknown_blocked_owner["owned_blocked"]["count"] is None
    assert unknown_blocked_owner["owned_blocked"]["observed_count"] == 0
    assert unknown_blocked_owner["wip"]["count"] == 0 and unknown_blocked_owner["counts_by_status"]["blocked"] == 1

    unavailable = API["collect_native"](responses(*[RuntimeError("offline")] * 4))
    assert unavailable["collection_status"] == "unavailable"
    assert unavailable["total_tasks"] is None and unavailable["tasks"] is None and unavailable["wip"] is None
    one_snapshot = API["collect_native"](responses(
        {"commit": "a", "branch": "main"}, rows, RuntimeError("second read failed"), {"commit": "a", "branch": "main"}))
    assert one_snapshot["collection_status"] == "partial" and one_snapshot["wip"]["count"] == 2

    annotated = [{**rows[0], "updated_at": NOW.isoformat(), "started_at": START.isoformat(),
                  "notes": "Continuity certified. Entered planning at " + START.isoformat(),
                  "metadata": {"status_entered_at": START.isoformat(), "continuity": "certified"}}]
    for report in (native_rows(annotated), native_rows(annotated, last_commit="b"),
                   native_rows(annotated, [{**annotated[0], "notes": "Another note"}])):
        age = report["tasks"][0]["phase_age"]
        assert age["classification"] == "unknown" and age["seconds"] is None and age["since"] is None
        assert report["wip"]["count"] == 1 and report["counts_by_status"]["planning"] == 1
    assert native_rows(annotated, last_commit="b")["collection_status"] == "partial"
    assert native_rows(annotated, [{**annotated[0], "status": "review"}])["collection_status"] == "partial"

    def forbidden(argv):
        raise AssertionError("paused collection invoked native store")

    paused = API["collect_native"](forbidden, paused=True)
    assert paused["collection_status"] == "paused" and paused["total_tasks"] is None


def delivery_boundaries():
    nodes = [pull(1, when=START), pull(2, when=START + timedelta(microseconds=1)), pull(3),
             pull(4, when=NOW + timedelta(microseconds=1)), pull(5, "CLOSED"), pull(6, "OPEN")]
    report = github(page(nodes[:3], 6, "page-two"), page(nodes[3:], 6))
    delivery = report["delivery"]
    assert report["collection_status"] == "complete" and delivery["count"] == 2
    assert {pr["number"] for pr in delivery["entries"]} == {2, 3}
    assert delivery["closed_unmerged_count"] == 1 and delivery["closed_unmerged"][0]["number"] == 5
    assert delivery["last_merge"]["number"] == 3 and delivery["seconds_since_last_merge"] == 0
    assert delivery["floor_met"] is True and {pr["number"] for pr in report["open_prs"]} == {6}
    old = github(page([pull(7, when=START - timedelta(days=1))], 1))
    assert old["delivery"]["count"] == 0 and old["delivery"]["floor_met"] is False
    assert old["delivery"]["seconds_since_last_merge"] == 90000
    no_commit = github(page([{**pull(8), "mergeCommit": None}, pull(9)], 2))
    assert no_commit["collection_status"] == "partial" and no_commit["source"]["inventory_complete"] is True
    assert no_commit["delivery"]["count"] is None and no_commit["delivery"]["floor_met"] is None
    assert no_commit["delivery"]["observed_count"] == 1 and no_commit["delivery"]["entries"][0]["number"] == 9
    assert no_commit["delivery"]["merge_commit_proof_complete"] is False
    assert no_commit["delivery"]["reported_unverified_candidates"][0]["number"] == 8
    assert no_commit["delivery"]["last_merge"] is None and no_commit["delivery"]["seconds_since_last_merge"] is None
    assert no_commit["delivery"]["closed_unmerged_count"] == 0
    for invalid_oid in ("a" * 39, "g" * 40):
        invalid_commit = github(page([{**pull(10), "mergeCommit": {"oid": invalid_oid}}], 1))
        assert invalid_commit["delivery"]["count"] is None and invalid_commit["delivery"]["floor_met"] is None
        assert invalid_commit["delivery"]["observed_count"] == 0
    incomplete_cases = (
        (page([pull(1)], 2, "next"), RuntimeError("page unavailable")),
        (page([pull(1)], 2, "next"), page([pull(1)], 2)),
        (page([pull(1)], 2, "next"), page([pull(2)], 3)),
        (page([pull(1)], 2, "next"), page([pull(2)], 2, "next")),
        (page([{**pull(1), "mergedAt": "not a date"}], 1),),
        ({**page([pull(1)], 1), "errors": [{"message": "partial response"}]},),
    )
    for pages in incomplete_cases:
        partial = github(*pages)
        assert partial["collection_status"] == "partial"
        assert partial["delivery"]["count"] is None and partial["delivery"]["floor_met"] is None
        assert partial["delivery"]["last_merge"] is None and partial["delivery"]["seconds_since_last_merge"] is None
        assert partial["errors"]
    lost_second = github(page([pull(1)], 2, "next"), RuntimeError("offline"))
    assert lost_second["delivery"]["observed_count"] == 1
    assert lost_second["delivery"]["last_merge_observed_candidate"]["number"] == 1
    failed = github(RuntimeError("offline"))
    assert failed["collection_status"] == "unavailable" and failed["pull_requests_total"] is None
    invalid = github(start=NOW, end=START)
    assert invalid["collection_status"] == "unavailable" and invalid["delivery"]["count"] is None


def security_issue(number, age):
    return {"number": number, "title": f"Security findings {number}",
            "url": f"https://github.com/example/repo/issues/{number}", "createdAt": (NOW - age).isoformat()}


def issues_page(nodes, total, cursor=None):
    return {"data": {"repository": {"issues": {
        "nodes": nodes, "totalCount": total,
        "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
    }}}}


def security(*pages):
    calls = []
    answer = responses(*pages)

    def run(argv):
        calls.append(argv)
        return answer(argv)

    report, errors = API["collect_security_issues"](run, repository="example/repo", observed_at=NOW)
    return report, errors, calls


def security_issues():
    week, hour = timedelta(days=8), timedelta(hours=1)
    report, errors, calls = security(issues_page([security_issue(4, week), security_issue(9, hour)], 2))
    assert report["collection_status"] == "complete" and errors == [], (report, errors)
    assert report["count"] == 2 and report["observed_count"] == 2, report
    assert report["issues"] == [
        {"number": 4, "title": "Security findings 4", "url": "https://github.com/example/repo/issues/4",
         "created_at": API["timestamp"](NOW - week), "age_seconds": week.total_seconds()},
        {"number": 9, "title": "Security findings 9", "url": "https://github.com/example/repo/issues/9",
         "created_at": API["timestamp"](NOW - hour), "age_seconds": hour.total_seconds()},
    ], report["issues"]
    assert len(calls) == 1 and calls[0][:3] == ["gh", "api", "graphql"], calls
    query = next(argument for argument in calls[0] if argument.startswith("query="))
    assert 'labels:["security"]' in query and "states:OPEN" in query, query
    assert "owner=example" in calls[0] and "name=repo" in calls[0], calls

    paged, errors, calls = security(issues_page([security_issue(4, week)], 2, "next"),
                                    issues_page([security_issue(9, hour)], 2))
    assert paged["collection_status"] == "complete" and paged["count"] == 2 and errors == [], paged
    assert "cursor=next" in calls[1], calls

    empty, errors, calls = security(issues_page([], 0))
    assert empty["collection_status"] == "complete" and empty["count"] == 0 and empty["issues"] == [], empty

    failed, errors, calls = security(RuntimeError("offline"))
    assert failed["collection_status"] == "unavailable", failed
    assert failed["count"] is None and failed["issues"] is None, failed
    assert errors and errors[0]["source"] == "github" and errors[0]["operation"] == "securityIssues", errors
    assert "offline" in errors[0]["message"], errors

    for pages in (
        (issues_page([security_issue(4, week)], 2, "next"), RuntimeError("second page unavailable")),
        (issues_page([security_issue(4, week)], 2, "next"), issues_page([security_issue(4, week)], 2)),
        (issues_page([security_issue(4, week)], 2),),
        ({**issues_page([security_issue(4, week)], 1), "errors": [{"message": "partial response"}]},),
        (issues_page([{**security_issue(4, week), "createdAt": "not a date"}], 1),),
        (issues_page([{**security_issue(4, week), "createdAt": (NOW - week).replace(tzinfo=None).isoformat()}], 1),),
        (issues_page([{**security_issue(4, week), "title": None}], 1),),
        (issues_page([{**security_issue(4, week), "number": "4"}], 1),),
        (issues_page([security_issue(4, week)], 2, "next"), issues_page([security_issue(9, hour)], 2, "next")),
        ({"data": {"repository": {"pullRequests": {}}}},),
        (["not an object"],),
        ({"data": {"repository": {"issues": {"nodes": "x", "totalCount": 1, "pageInfo": {}}}}},),
        ({"data": {"repository": {"issues": {"nodes": [], "totalCount": 0, "pageInfo": {"endCursor": None}}}}},),
    ):
        partial, errors, calls = security(*pages)
        assert partial["collection_status"] in {"partial", "unavailable"}, (pages, partial)
        assert partial["count"] is None and errors, (pages, partial, errors)
        assert all(item["operation"] == "securityIssues" for item in errors), errors
    lost, errors, calls = security(issues_page([security_issue(4, week)], 2, "next"), RuntimeError("offline"))
    assert lost["collection_status"] == "partial" and lost["observed_count"] == 1, lost
    assert [item["number"] for item in lost["issues"]] == [4], lost


def cli_security_issues():
    with tempfile.TemporaryDirectory(prefix="pipeline-health-security-regression-") as directory:
        root = Path(directory)
        tools = root / "tools"
        tools.mkdir()
        shutil.copy2(SOURCE, tools / SOURCE.name)
        shutil.copy2(SOURCE.with_name("pipeline_review_usage.py"), tools / "pipeline_review_usage.py")
        created = datetime.now(timezone.utc) - timedelta(days=10)
        pulls = {"data": {"repository": {"pullRequests": {
            "nodes": [], "totalCount": 0, "pageInfo": {"hasNextPage": False, "endCursor": None}}}}}
        issues = {"data": {"repository": {"issues": {
            "nodes": [{"number": 12, "title": "Security findings on main",
                       "url": "https://github.com/example/repo/issues/12", "createdAt": created.isoformat()}],
            "totalCount": 1, "pageInfo": {"hasNextPage": False, "endCursor": None}}}}}
        gh = root / "gh"
        gh.write_text(f"#!{sys.executable}\nimport json, os, sys\n"
                      "query = next(argument for argument in sys.argv if argument.startswith('query='))\n"
                      "if 'issues(' in query and os.path.exists('fail-issues'):\n"
                      "    print('fixture issues query refused', file=sys.stderr)\n"
                      "    raise SystemExit(1)\n"
                      f"print(json.dumps({issues!r} if 'issues(' in query else {pulls!r}))\n")
        gh.chmod(0o755)
        environment = {"PATH": str(root) + os.pathsep + os.defpath, "PYTHONDONTWRITEBYTECODE": "1"}

        def collect():
            result = subprocess.run([sys.executable, str(tools / SOURCE.name), "--repo", "example/repo", "--native-paused"],
                                    cwd=root, env=environment, text=True, capture_output=True, timeout=15)
            assert result.returncode == 2, result
            return json.loads(result.stdout)

        report = collect()
        listed = report["github"]["security_issues"]
        assert report["github"]["collection_status"] == "complete", report["github"]
        assert listed["collection_status"] == "complete" and listed["count"] == 1, listed
        assert listed["issues"][0]["number"] == 12 and listed["issues"][0]["url"].endswith("/issues/12"), listed
        assert timedelta(days=10).total_seconds() <= listed["issues"][0]["age_seconds"] <= timedelta(days=10, minutes=1).total_seconds(), listed
        assert not [item for item in report["errors"] if item["source"] == "github"], report["errors"]

        (root / "fail-issues").write_text("")
        report = collect()
        listed = report["github"]["security_issues"]
        assert report["github"]["collection_status"] == "partial", report["github"]
        assert listed["collection_status"] == "unavailable" and listed["count"] is None and listed["issues"] is None, listed
        assert report["github"]["source"]["inventory_complete"] is True, report["github"]["source"]
        assert report["reviews"]["collection_status"] == "complete", report["reviews"]
        failures = [item for item in report["errors"] if item["source"] == "github"]
        assert [item["operation"] for item in failures] == ["securityIssues"], report["errors"]
        assert "fixture issues query refused" in failures[0]["message"], failures


def cli_failure_isolation():
    with tempfile.TemporaryDirectory(prefix="pipeline-health-cli-regression-") as directory:
        root = Path(directory)
        tools, binaries = root / "tools", root / "bin"
        tools.mkdir()
        binaries.mkdir()
        shutil.copy2(SOURCE, tools / SOURCE.name)
        shutil.copy2(SOURCE.with_name("pipeline_review_usage.py"), tools / "pipeline_review_usage.py")
        marker = root / "native-calls"
        launcher = tools / "work-state"
        launcher.write_text("#!/usr/bin/env python3\nimport json, pathlib, sys\n"
                            "with pathlib.Path('native-calls').open('a') as out: out.write('native call\\n')\n"
                            "if '--readonly' not in sys.argv: raise SystemExit(93)\n"
                            "print(json.dumps({'commit':'a','branch':'main'} if 'vc' in sys.argv else "
                            "[{'id':'active','title':'Active','status':'in_progress','assignee':'author'}]))\n")
        launcher.chmod(0o700)
        gh = binaries / "gh"
        gh.write_text("#!/usr/bin/env python3\nimport sys\nprint('fixture GitHub unavailable', file=sys.stderr)\nraise SystemExit(1)\n")
        gh.chmod(0o700)
        environment = {**os.environ, "PATH": str(binaries) + os.pathsep + os.environ["PATH"]}
        for paused in (True, False):
            before = datetime.now(timezone.utc)
            result = subprocess.run([sys.executable, str(tools / SOURCE.name), "--repo", "example/repo",
                                     *(["--native-paused"] if paused else [])], cwd=root, env=environment,
                                    text=True, capture_output=True, timeout=15)
            after = datetime.now(timezone.utc)
            assert result.returncode == 2, result
            report = json.loads(result.stdout)
            assert before <= API["parse_time"](report["observed_at"]) <= API["parse_time"](report["completed_at"]) <= after
            assert API["parse_time"](report["window"]["end_inclusive"]) - API["parse_time"](report["window"]["start_exclusive"]) == timedelta(hours=1)
            assert report["github"]["collection_status"] == "unavailable"
            assert report["github"]["delivery"]["count"] is None
            assert report["reviews"]["collection_status"] == "partial"
            assert report["reviews"]["counts"]["completed_in_window"] is None
            assert report["reviews"]["coverage"]["observed_known_completed_in_window"] == 0
            if paused:
                assert not marker.exists() and report["native"]["collection_status"] == "paused"
            else:
                assert marker.exists() and report["native"]["collection_status"] == "complete"
                assert report["native"]["wip"]["count"] == 1
        gh.write_text("#!/usr/bin/env python3\nimport json\n"
                      "print(json.dumps({'data':{'repository':{'pullRequests':{'nodes':[],"
                      "'totalCount':0,'pageInfo':{'hasNextPage':False,'endCursor':None}}}}}))\n")
        empty = subprocess.run([sys.executable, "-P", str(tools / SOURCE.name), "--repo", "example/repo",
                                "--native-paused"], cwd=root, env=environment,
                               text=True, capture_output=True, timeout=15)
        assert empty.returncode == 2
        empty_report = json.loads(empty.stdout)
        assert empty_report["github"]["source"]["inventory_complete"]
        assert empty_report["reviews"]["collection_status"] == "complete"
        assert empty_report["reviews"]["counts"]["completed_in_window"] == 0


def cli_readonly_startup():
    with tempfile.TemporaryDirectory(prefix="pipeline-health-startup-regression-") as directory:
        root = Path(directory)
        tools = root / "tools"
        tools.mkdir()
        shutil.copy2(SOURCE, tools / SOURCE.name)
        shutil.copy2(SOURCE.with_name("pipeline_review_usage.py"), tools / "pipeline_review_usage.py")
        payload = {"data": {"repository": {"pullRequests": {
            "nodes": [], "totalCount": 0, "pageInfo": {"hasNextPage": False, "endCursor": None}}}}}
        gh = root / "gh"
        gh.write_text(f"#!{sys.executable}\nprint({json.dumps(payload)!r})\n")
        gh.chmod(0o755)

        def snapshot():
            return {str(path.relative_to(root)): (path.lstat().st_mode,
                    str(path.readlink()) if path.is_symlink() else path.read_bytes() if path.is_file() else None)
                    for path in root.rglob("*")}

        before = snapshot()
        result = subprocess.run([sys.executable, str(tools / SOURCE.name), "--repo", "example/repo",
                                 "--native-paused"], cwd=root,
                                env={"PATH": str(root) + os.pathsep + os.defpath, "PYTHONSAFEPATH": "1"},
                                text=True, capture_output=True, timeout=15)
        assert result.returncode == 2
        assert json.loads(result.stdout)["reviews"]["collection_status"] == "complete"
        assert snapshot() == before, "read-only startup changed the checkout inventory"


def runner_boundaries():
    with tempfile.TemporaryDirectory(prefix="pipeline-health-runner-regression-") as directory:
        root = Path(directory)
        runner = API["JsonRunner"](root, timeout=3, budget=15, output_limit=1024)
        for code in ("print('not JSON')", "print('x' * 2048)",
                     "import os,time,pathlib; pathlib.Path('pid').write_text(str(os.getpid())); time.sleep(60)"):
            try:
                runner([sys.executable, "-c", code])
            except RuntimeError:
                pass
            else:
                raise AssertionError("invalid/oversized/timed-out source accepted")
        pid = int((root / "pid").read_text())
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            pass
        else:
            raise AssertionError("timed-out owned process is still present")
        assert runner([sys.executable, "-c", "print('{\"still_usable\":true}')"])["still_usable"] is True
        try:
            runner([sys.executable, "-c", "import sys; print('Authorization: Bearer private-value', file=sys.stderr); sys.exit(1)"])
        except RuntimeError as exc:
            assert "private-value" not in str(exc) and "exited 1" in str(exc)
        else:
            raise AssertionError("nonzero source command accepted")


def cli_exit_metadata():
    with tempfile.TemporaryDirectory(prefix="pipeline-health-exit-regression-") as directory:
        root = Path(directory)
        gh = root / "gh"
        gh.write_text(f"#!{sys.executable}\nimport sys\n"
                      "print('source command exited 1: xy AWS_AUTH_RETRIES=1 token=xy', file=sys.stderr)\n"
                      "sys.exit(1)\n")
        gh.chmod(0o755)
        environment = {"PATH": str(root) + os.pathsep + os.defpath,
                       "AWS_AUTH_RETRIES": "1", "PIPELINE_TEST_TOKEN": "xy",
                       "PYTHONDONTWRITEBYTECODE": "1"}
        result = subprocess.run([sys.executable, str(SOURCE), "--repo", "example/repo", "--native-paused"],
                                cwd=root, env=environment, text=True, capture_output=True, timeout=15)
        assert result.returncode == 2
        report = json.loads(result.stdout)
        source_message = report["github"]["errors"][0]["message"]
        aggregate_message = next(item["message"] for item in report["errors"] if item["source"] == "github")
        for message in (source_message, aggregate_message):
            prefix, detail = message.split(": ", 1)
            assert prefix == "source command exited 1"
            assert "1" not in detail and "xy" not in detail
            assert "AWS_AUTH_RETRIES=[REDACTED]" in detail and "token=[REDACTED]" in detail


def process_state(pid):
    result = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], stdin=subprocess.DEVNULL,
                            capture_output=True, text=True, timeout=10)
    state = result.stdout.strip()
    return state if result.returncode == 0 and state else None


def wait_for_zombie(pid):
    end = time.monotonic() + 15
    while not (process_state(pid) or "").startswith("Z"):
        assert time.monotonic() < end, f"source child {pid} never became an exited, unreaped zombie"
        time.sleep(0.01)


def ended_within(pid, seconds):
    end = time.monotonic() + seconds
    while True:
        state = process_state(pid)
        if state is None or state.startswith("Z"):
            return True
        if time.monotonic() >= end:
            return False
        time.sleep(0.01)


@contextmanager
def runner_syscalls(killpg, *, exited_before_read=False):
    namespace = API["JsonRunner"].__call__.__globals__
    original_os, original_subprocess = namespace["os"], namespace["subprocess"]

    class Processes:
        child = None

        def __getattr__(self, name):
            return getattr(original_subprocess, name)

        def Popen(self, *args, **kwargs):
            self.child = original_subprocess.Popen(*args, **kwargs)
            return self.child

    processes = Processes()

    class Syscalls:
        def __getattr__(self, name):
            return getattr(original_os, name)

        def killpg(self, group, sig):
            return killpg(processes.child, group, sig)

        def read(self, fd, size):
            if exited_before_read:
                wait_for_zombie(processes.child.pid)
            return original_os.read(fd, size)

    namespace["subprocess"], namespace["os"] = processes, Syscalls()
    try:
        yield processes
    finally:
        namespace["subprocess"], namespace["os"] = original_subprocess, original_os
        if processes.child is not None and processes.child.returncode is None:
            processes.child.kill()
            processes.child.wait(timeout=10)


def runner_released_group_identity():
    # Reuse is injected at the syscall boundary; its replacement is a real owned process.
    victim = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
                              stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, start_new_session=True)

    def killpg(child, group, sig):
        if child is not None and group == child.pid and child.returncode is not None:
            try:
                os.waitpid(child.pid, os.WNOHANG)
            except ChildProcessError:
                # The original identity is actually released, not merely an exited child.
                os.killpg(victim.pid, sig)
                victim.wait(timeout=5)
            else:
                raise AssertionError("fixture expected a reaped source child")
        else:
            os.killpg(group, sig)

    try:
        with runner_syscalls(killpg):
            result = API["JsonRunner"](Path.cwd(), timeout=2, budget=5)(
                [sys.executable, "-c", "print('{\"ok\":true}')"])
        assert result == {"ok": True}
        assert victim.poll() is None, "successful collection killed a replacement process group"
    finally:
        if victim.poll() is None:
            victim.kill()
        victim.wait(timeout=5)


def runner_refused_group_of_exited_leader():
    calls = []

    def killpg(child, group, sig):
        calls.append("after reap" if group != child.pid or child.returncode is not None else "injected")
        assert calls[-1] == "injected", "signalled a group after its leader was reaped"
        try:
            os.killpg(group, sig)
        except PermissionError:
            calls[-1] = "kernel"
            raise
        raise PermissionError(errno.EPERM, os.strerror(errno.EPERM))

    with tempfile.TemporaryDirectory(prefix="pipeline-health-exited-leader-") as directory, \
            runner_syscalls(killpg, exited_before_read=True) as processes:
        runner = API["JsonRunner"](Path(directory), timeout=10, budget=30, output_limit=1024)
        try:
            runner([sys.executable, "-c", "print('x' * 2048)"])
        except RuntimeError as exc:
            assert "exceeded byte limit" in str(exc), exc
        else:
            raise AssertionError("oversized source accepted")
        child = processes.child
        assert child.returncode is not None, "the exited leader of a refused group was left unreaped"
        assert child.stdout.closed and child.stderr.closed, "a refused group left its pipes open"
        assert "after reap" not in calls, f"signalled a group after its leader was reaped: {calls}"
        assert len(calls) == 1, f"expected exactly one killpg, saw {calls}"
    return calls


def the_kernel_refuses_killpg_on_a_group_whose_only_member_is_a_zombie():
    only_on_macos("killpg on a group whose leader exited unreaped returns EPERM only on Darwin")
    calls = runner_refused_group_of_exited_leader()
    assert calls == ["kernel"], f"the kernel did not refuse a group whose only member is a zombie: {calls}"


def runner_refused_group_of_live_leader():
    def killpg(child, group, sig):
        assert group == child.pid and child.returncode is None, "signalled a group after its leader was reaped"
        raise PermissionError(errno.EPERM, os.strerror(errno.EPERM))

    with tempfile.TemporaryDirectory(prefix="pipeline-health-live-leader-") as directory, \
            runner_syscalls(killpg) as processes:
        runner = API["JsonRunner"](Path(directory), timeout=0.5, budget=60)
        runner.exit_grace = 0.5
        try:
            runner([sys.executable, "-c", "import time; time.sleep(30)"])
        except RuntimeError as exc:
            assert "refused SIGKILL" in str(exc) and "sleep" not in str(exc), exc
        else:
            raise AssertionError("a live group that refused SIGKILL was accepted")
        child = processes.child
        assert child.poll() is None, "the refusing leader was not live, so the row tested nothing"
        assert child.stdout.closed and child.stderr.closed, "a refused group left its pipes open"


def runner_refused_group_of_exiting_leader():
    calls, timers = [], []

    def killpg(child, group, sig):
        calls.append("after reap" if group != child.pid or child.returncode is not None else "refused")
        assert calls[-1] == "refused", "signalled a group after its leader was reaped"
        timer = threading.Timer(1, child.kill)
        timers.append(timer)
        timer.start()
        raise PermissionError(errno.EPERM, os.strerror(errno.EPERM))

    with tempfile.TemporaryDirectory(prefix="pipeline-health-exiting-leader-") as directory, \
            runner_syscalls(killpg) as processes:
        # The call's deadline is the whole budget, so no time is left when the refusal comes.
        runner = API["JsonRunner"](Path(directory), timeout=0.5, budget=0.5)
        # A loaded Mac can run the 1 s timer seconds late. Six times the runner's grace leaves room
        # for that, and a grace of 0 still fails. The leader sleeps past the grace: only the timer ends it.
        runner.exit_grace *= 6
        try:
            runner([sys.executable, "-c", "import time; time.sleep(60)"])
        except RuntimeError as exc:
            assert str(exc) == "source command timed out", exc
        else:
            raise AssertionError("timed-out source accepted")
        finally:
            for timer in timers:
                timer.cancel()
                timer.join()
        assert processes.child.returncode is not None, "a leader that exited during the grace was left unreaped"
        assert calls == ["refused"], f"expected exactly one killpg, saw {calls}"


def runner_ends_descendants_of_exited_leader():
    def killpg(child, group, sig):
        os.killpg(group, sig)

    leader = ("import os, pathlib, time\n"
              "pid = os.fork()\n"
              "if pid == 0:\n"
              "    quiet = os.open(os.devnull, os.O_RDWR)\n"
              "    for fd in (0, 1, 2):\n"
              "        os.dup2(quiet, fd)\n"
              "    time.sleep(60)\n"
              "    os._exit(0)\n"
              "pathlib.Path('descendant').write_text(str(pid))\n"
              "print('x' * 2048)\n")
    with tempfile.TemporaryDirectory(prefix="pipeline-health-descendant-") as directory, \
            runner_syscalls(killpg, exited_before_read=True) as processes:
        record = Path(directory) / "descendant"
        try:
            runner = API["JsonRunner"](Path(directory), timeout=10, budget=30, output_limit=1024)
            try:
                runner([sys.executable, "-c", leader])
            except RuntimeError as exc:
                assert "exceeded byte limit" in str(exc), exc
            else:
                raise AssertionError("oversized source accepted")
            assert ended_within(int(record.read_text()), 10), "a descendant of an exited leader outlived the call"
        finally:
            if record.exists():
                descendant = int(record.read_text())
                try:
                    if os.getpgid(descendant) == processes.child.pid and not ended_within(descendant, 0):
                        os.kill(descendant, signal.SIGKILL)
                except ProcessLookupError:
                    pass


def only_on_macos_skips_by_name_off_darwin():
    platform = sys.platform
    try:
        sys.platform = "linux"
        try:
            only_on_macos("a probe reason")
        except Skipped as skipped:
            assert str(skipped) == "macOS only: a probe reason", skipped
        else:
            raise AssertionError("only_on_macos let a row run off Darwin")
        sys.platform = "darwin"
        try:
            returned = only_on_macos("a probe reason")
        except Skipped as skipped:
            raise AssertionError(f"only_on_macos skipped a row on Darwin: {skipped}") from None
        assert returned is None, returned
    finally:
        sys.platform = platform


def a_failed_row_fails_the_run_whatever_rows_skip():
    def passes():
        pass

    def fails():
        raise AssertionError("broken")

    def crashes():
        raise RuntimeError("crashed")

    def exits():
        raise SystemExit(0)

    def ends():
        sys.exit()

    def skips():
        raise Skipped("macOS only: a probe reason")

    def run(rows):
        try:
            return run_rows(rows)
        except SystemExit as error:
            return f"ended by {error!r}"

    printed, errors = io.StringIO(), io.StringIO()
    with redirect_stdout(printed), redirect_stderr(errors):
        statuses = [run([skips, fails]), run([crashes, skips]), run([passes, skips]), run([exits, passes]), run([ends, passes])]
    assert statuses == [1, 1, 0, 1, 1], statuses
    assert printed.getvalue().splitlines() == [
        "skip skips: macOS only: a probe reason", "FAIL fails: AssertionError: broken", "0 passed, 1 failed, 1 skipped",
        "FAIL crashes: RuntimeError: crashed", "skip skips: macOS only: a probe reason", "0 passed, 1 failed, 1 skipped",
        "ok passes", "skip skips: macOS only: a probe reason", "1 passed, 0 failed, 1 skipped",
        "FAIL exits: SystemExit: exit code 0", "ok passes", "1 passed, 1 failed, 0 skipped",
        "FAIL ends: SystemExit: exit code None", "ok passes", "1 passed, 1 failed, 0 skipped",
    ], printed.getvalue()
    assert errors.getvalue().count("Traceback") == 4, errors.getvalue()
    with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
        long_run = run([passes] * len(TESTS) + [fails])
    assert long_run == 1, f"one failed row in a run as long as TESTS ended {long_run!r}"


def run_rows(rows):
    passed, failed, skipped = [], [], []
    for row in rows:
        try:
            row()
        except Skipped as reason:
            skipped.append(row.__name__)
            print(f"skip {row.__name__}: {reason}")
        except (Exception, SystemExit) as error:
            failed.append(row.__name__)
            detail = f"exit code {error.code!r}" if isinstance(error, SystemExit) else error
            print(f"FAIL {row.__name__}: {type(error).__name__}: {detail}", flush=True)
            traceback.print_exception(error)
        else:
            passed.append(row.__name__)
            print(f"ok {row.__name__}")
    print(f"{len(passed)} passed, {len(failed)} failed, {len(skipped)} skipped")
    return 1 if failed else 0


TESTS = [
    only_on_macos_skips_by_name_off_darwin,
    native_uncertainty,
    delivery_boundaries,
    security_issues,
    cli_security_issues,
    cli_failure_isolation,
    cli_readonly_startup,
    runner_boundaries,
    cli_exit_metadata,
    runner_released_group_identity,
    runner_refused_group_of_exited_leader,
    the_kernel_refuses_killpg_on_a_group_whose_only_member_is_a_zombie,
    runner_refused_group_of_live_leader,
    runner_refused_group_of_exiting_leader,
    runner_ends_descendants_of_exited_leader,
    a_failed_row_fails_the_run_whatever_rows_skip,
]


if __name__ == "__main__":
    a_failed_row_fails_the_run_whatever_rows_skip()
    sys.exit(run_rows(TESTS))
