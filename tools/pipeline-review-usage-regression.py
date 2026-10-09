#!/usr/bin/env python3
"""Deterministic observable regressions for uncertain GitHub review evidence."""

from copy import deepcopy
from datetime import datetime, timezone
from urllib.parse import parse_qs, urlsplit

from pipeline_review_usage import collect_review_usage


NOW = datetime(2026, 1, 2, 12, tzinfo=timezone.utc)
START = datetime(2026, 1, 2, 11, tzinfo=timezone.utc)
HEAD = "a" * 40
OLD = "b" * 40
RUN_A = "11111111-2222-3333-4444-555555555555"
RUN_B = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
BOT = {"login": "coderabbitai[bot]"}
PR = {"number": 1, "state": "OPEN", "updatedAt": "2026-01-02T11:30:00Z", "headRefOid": HEAD}


def status(identity, at, description="Review completed"):
    return {"id": identity, "creator": BOT, "context": "CodeRabbit", "state": "success",
            "description": description, "created_at": at}


def review(identity, run, at, head=HEAD):
    return {"id": identity, "user": BOT, "state": "COMMENTED", "commit_id": head,
            "submitted_at": at,
            "body": f"**Actionable comments posted: 0**\n\n**Run ID**: `{run}`\n"}


def reply(identity, invocation, start, end):
    return {"id": identity, "user": BOT, "created_at": start, "updated_at": end,
            "body": f"<!-- CodeRabbit review command invocation: v2:{invocation * 64} -->\nFull review finished.\n"}


class Forge:
    def __init__(self, surfaces):
        self.surfaces = surfaces

    def __call__(self, argv):
        # A mutation must fail the exercised read-only consumer boundary.
        assert argv[:4] == ["gh", "api", "--method", "GET"], argv
        parsed = urlsplit(argv[4])
        path = parsed.path.removeprefix("repos/example/project/")
        page = int(parse_qs(parsed.query)["page"][0])
        value = self.surfaces.get(path, [])
        if isinstance(value, dict):
            value = value.get(page, [])
        else:
            value = value[(page - 1) * 100:page * 100]
        if isinstance(value, Exception):
            raise value
        return deepcopy(value)


def collect(surfaces, prs=None):
    return collect_review_usage(Forge(surfaces), repository="example/project",
                                pull_requests=prs or [PR], window_start=START, observed_at=NOW)


def deduplicates_runs_not_status_rows():
    first = review(1, RUN_A, "2026-01-02T11:10:00Z")
    result = collect({"pulls/1/reviews": [first, review(2, RUN_A, "2026-01-02T11:10:01Z"),
                                             review(3, RUN_B, "2026-01-02T11:30:00Z")],
                      f"statuses/{HEAD}": [status(10, "2026-01-02T11:10:03Z"),
                                            status(11, "2026-01-02T11:15:00Z"),
                                            status(12, "2026-01-02T11:30:04Z")],
                      "issues/1/comments": [reply(20, "a", "2026-01-02T11:01:00Z", "2026-01-02T11:10:05Z")]})
    assert result["counts"]["completed_in_window"] == 2, result
    assert len(result["events"]) == 2
    assert result["counts"]["admitted_in_window"] is None
    assert result["remaining"] is None


def separates_completion_and_unknown_charge_window():
    result = collect({"pulls/1/reviews": [review(1, RUN_A, "2026-01-02T10:59:00Z"),
                                             review(2, RUN_B, "2026-01-02T11:59:50Z")],
                      f"statuses/{HEAD}": [status(10, "2026-01-02T11:00:00Z"),
                                            status(11, "2026-01-02T12:00:00Z"),
                                            status(12, "2026-01-02T12:00:01Z")]})
    assert result["counts"]["completed_in_window"] == 1, result
    assert result["counts"]["admission_time_unknown"] == 2
    assert all(event["admitted_at"] is None for event in result["events"])
    assert result["counts"]["admitted_in_window"] is None


def paused_green_does_not_manufacture_current_head_run():
    result = collect({"pulls/1/reviews": [review(1, RUN_A, "2026-01-02T11:10:00Z", OLD),
                                             {"id": 2, "user": BOT, "state": "COMMENTED", "body": "",
                                              "commit_id": HEAD, "submitted_at": "2026-01-02T11:20:00Z"}],
                      "issues/1/comments": [{"id": 3, "user": BOT, "created_at": "2026-01-01T10:00:00Z",
                                             "updated_at": "2026-01-02T11:25:00Z",
                                             "body": '<!-- review paused by coderabbit.ai -->\n<!-- final_review_risk_coverage:{"coveredCommitId":"' + OLD + '","kind":"reviewed"} -->'}],
                      f"statuses/{OLD}": [status(10, "2026-01-02T11:10:04Z")],
                      f"statuses/{HEAD}": [status(11, "2026-01-02T11:25:01Z"),
                                            status(12, "2026-01-02T11:26:00Z", "Review rate limited"),
                                            status(13, "2026-01-02T11:27:00Z", "Review skipped: draft pull request")]})
    assert result["counts"]["completed_in_window"] == 1, result
    assert {event["head"] for event in result["events"]} == {OLD}
    assert any(item.get("head") == HEAD for item in result["ambiguities"])
    assert any(item["head"] == OLD for item in result["coverage"]["rewritten_heads"])


def clean_review_without_review_object_and_pagination():
    nonreview = [dict(status(i, "2026-01-02T11:00:00Z"), context="CI") for i in range(100)]
    finished = reply(500, "a", "2026-01-02T10:59:00Z", "2026-01-02T11:05:05Z")
    result = collect({"issues/1/comments": [finished, finished],
                      f"statuses/{HEAD}": {1: nonreview, 2: [status(200, "2026-01-02T11:05:00Z"),
                                                                  status(201, "2026-01-02T11:05:01Z")]}})
    assert result["counts"]["completed_in_window"] == 1, result
    assert result["events"][0]["completed_at"] == "2026-01-02T11:05:00Z"
    assert result["events"][0]["run_id"] is None
    assert result["events"][0]["admitted_at"] is None


def incomplete_sources_never_become_zero_usage():
    rows = [dict(status(i, "2026-01-02T11:00:00Z"), context="CI") for i in range(100)]
    result = collect({f"statuses/{HEAD}": {1: rows, 2: RuntimeError("sensitive transport output")}})
    assert result["collection_status"] == "partial", result
    assert result["counts"]["completed_in_window"] is None
    assert result["errors"] and "sensitive transport output" not in str(result)
    unavailable = collect({path: {1: RuntimeError("unreachable")} for path in
                           ["pulls/1/commits", "pulls/1/reviews", "pulls/1/comments", "issues/1/comments", f"statuses/{HEAD}"]})
    assert unavailable["collection_status"] == "unavailable"
    assert unavailable["counts"]["completed_in_window"] is None


def overlapping_invocations_do_not_double_charge():
    result = collect({"issues/1/comments": [reply(1, "a", "2026-01-02T11:00:00Z", "2026-01-02T11:10:00Z"),
                                           reply(2, "b", "2026-01-02T11:01:00Z", "2026-01-02T11:10:01Z")],
                      f"statuses/{HEAD}": [status(10, "2026-01-02T11:09:00Z")]})
    assert result["counts"]["completed_in_window"] == 0, result
    assert all(event["counted_as"]["completed_in_window"] is None for event in result["events"])
    assert any(item.get("status_id") == 10 for item in result["ambiguities"])


def main():
    deduplicates_runs_not_status_rows()
    separates_completion_and_unknown_charge_window()
    paused_green_does_not_manufacture_current_head_run()
    clean_review_without_review_object_and_pagination()
    incomplete_sources_never_become_zero_usage()
    overlapping_invocations_do_not_double_charge()
    print("PASS: run dedupe, completion/charge windows, paused green, clean review without review object, pagination/source failures, ambiguous overlapping identities")


if __name__ == "__main__":
    main()
