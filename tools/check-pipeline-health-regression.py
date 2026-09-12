#!/usr/bin/env python3
"""Defend health-report uncertainty, delivery windows, pagination and read-only failure boundaries."""

from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile


SOURCE = Path(__file__).resolve().with_name("check-pipeline-health")
API = runpy.run_path(str(SOURCE))
NOW = datetime(2026, 9, 12, 15, tzinfo=timezone.utc)
START = NOW - timedelta(hours=1)


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
        empty = subprocess.run([sys.executable, str(tools / SOURCE.name), "--repo", "example/repo",
                                "--native-paused"], cwd=root, env=environment,
                               text=True, capture_output=True, timeout=15)
        assert empty.returncode == 2
        empty_report = json.loads(empty.stdout)
        assert empty_report["github"]["source"]["inventory_complete"]
        assert empty_report["reviews"]["collection_status"] == "complete"
        assert empty_report["reviews"]["counts"]["completed_in_window"] == 0


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


def runner_released_group_identity():
    # Reuse is injected at the syscall boundary; its replacement is a real owned process.
    namespace = API["JsonRunner"].__call__.__globals__
    original_os, original_subprocess = namespace["os"], namespace["subprocess"]
    victim = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
                              stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, start_new_session=True)

    class Processes:
        child = None

        def __getattr__(self, name):
            return getattr(original_subprocess, name)

        def Popen(self, *args, **kwargs):
            self.child = original_subprocess.Popen(*args, **kwargs)
            return self.child

    processes = Processes()

    class RecycledGroups:
        def __getattr__(self, name):
            return getattr(original_os, name)

        def killpg(self, group, sig):
            child = processes.child
            if child is not None and group == child.pid and child.returncode is not None:
                try:
                    original_os.waitpid(child.pid, original_os.WNOHANG)
                except ChildProcessError:
                    # The original identity is actually released, not merely an exited child.
                    original_os.killpg(victim.pid, sig)
                    victim.wait(timeout=5)
                else:
                    raise AssertionError("fixture expected a reaped source child")
            else:
                original_os.killpg(group, sig)

    namespace["subprocess"], namespace["os"] = processes, RecycledGroups()
    try:
        result = API["JsonRunner"](Path.cwd(), timeout=2, budget=5)(
            [sys.executable, "-c", "print('{\"ok\":true}')"])
        assert result == {"ok": True}
        assert victim.poll() is None, "successful collection killed a replacement process group"
    finally:
        namespace["subprocess"], namespace["os"] = original_subprocess, original_os
        if victim.poll() is None:
            victim.kill()
        victim.wait(timeout=5)


def main():
    native_uncertainty()
    delivery_boundaries()
    cli_failure_isolation()
    runner_boundaries()
    runner_released_group_identity()
    print("PASS: native uncertainty/ownership/drift/pause, complete-or-unknown delivery windows/pagination, CLI clock/failure isolation, bounded reaped source commands")


if __name__ == "__main__":
    main()
