"""Read-only CodeRabbit run evidence; deliberately not a quota meter or merge gate.

The public Metrics API reports finalized PR aggregates, not hourly admissions.
GitHub statuses are immutable observations, but status IDs are not review run IDs.
Count a completed run only when provider output identifies it and a completed
status corroborates it. Mutable walkthroughs and command replies alone cannot
supply a completion timestamp. Unknown admission and billing remain unknown.
"""

from datetime import datetime, timezone
import json
import os
import re


DOCS = "https://docs.coderabbit.ai/"
RUN = re.compile(r"^\*\*Run ID\*\*:\s*`([0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12})`\s*$", re.M)
INVOCATION = re.compile(r"<!-- CodeRabbit review command invocation: (v2:[0-9a-f]{64}) -->")
SHA = re.compile(r"[0-9a-fA-F]{40}")
COVERAGE = re.compile(r'<!-- final_review_risk_coverage:(\{[^\n]*\}) -->')


def _time(value):
    if not isinstance(value, str):
        return None
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return result.astimezone(timezone.utc) if result.tzinfo else None
    except ValueError:
        return None


def _iso(value):
    return value.isoformat().replace("+00:00", "Z") if value else None


def _bot(row, key="user"):
    actor = row.get(key)
    login = actor.get("login") if isinstance(actor, dict) else None
    return isinstance(login, str) and login.lower() == "coderabbitai[bot]"


def _evidence(surface, row, head=None):
    return {"surface": surface, "id": row.get("id"), "head": head,
            "url": row.get("html_url") or row.get("url"),
            "created_at": row.get("created_at"), "updated_at": row.get("updated_at"),
            "submitted_at": row.get("submitted_at")}


def _diagnostic(body):
    # Match provider headings, not arbitrary quoted findings containing these words.
    lower = body.lower()
    if re.search(r"^\s*(?:>\s*)?Review rate limited\.?\s*$", body, re.M | re.I) or "your included review limit is currently reached" in lower:
        return "rate_limit_diagnostic_not_a_review"
    if "review paused by coderabbit.ai" in lower or "## reviews paused" in lower:
        return "paused_walkthrough_not_a_new_review"
    if re.search(r"^\s*(?:>\s*)?(?:#+\s*)?Review skipped\b", body, re.M | re.I):
        return "skipped_review_diagnostic"
    return None


def collect_review_usage(run_json, *, repository: str, pull_requests: list[dict],
                         window_start: datetime, observed_at: datetime,
                         baseline_per_hour: int = 10) -> dict:
    """Collect all pages in an explicit open/recent-PR GitHub observation scope.

    ``run_json`` is the caller's bounded, cwd-bound argv runner. No provider key
    is read or transmitted: the documented API has no suitable hourly meter.
    Only presence of the documented environment variable is inspected. No local
    credential stores, native task commands, durable cache or mutations are used.
    """
    report = {
        "collection_status": "complete", "provider": "coderabbit",
        "baseline": {"reviews_per_hour": baseline_per_hour, "source": "owner",
                     "scope": "shared total, not remaining"},
        "preferred_api": {
            "status": "unsupported",
            "reason": "The documented public API exposes Enterprise finalized-PR metrics, not individual hourly admissions or a remaining-review meter. Review Usage is documented as a dashboard, not a public API. No undocumented endpoint is called.",
            "documentation_checked_on": "2026-09-12",
            "authentication": {"CODERABBIT_API_KEY_present": bool(os.environ.get("CODERABBIT_API_KEY")),
                               "credential_stores_inspected": False,
                               "github_credentials_reused": False},
            "scope": "documented public CodeRabbit API; no account telemetry collected",
            "sources": [DOCS + "api/index", DOCS + "api-reference/metrics-data-api",
                        DOCS + "management/review-usage-dashboard", DOCS + "llms.txt"],
        },
        "measurement_scope": "repository GitHub known-run proxy in selected PRs; not account-authoritative",
        "window": {"start_exclusive": _iso(window_start), "end_inclusive": _iso(observed_at)},
        "counts": {}, "remaining": None, "events": [], "excluded": [], "ambiguities": [],
        "coverage": {
            "collection_started_at": _iso(datetime.now(timezone.utc)),
            "selection": "OPEN or updatedAt >= window start; missing/invalid updatedAt included",
            "inventory_count": len(pull_requests), "selected_prs": [], "omitted_prs": [],
            "surfaces": [], "heads": [], "rewritten_heads": [],
            "outside_repository_usage": "unknown", "account_usage": "unknown",
            "unobserved_heads": "Rewritten-away heads without surviving review references and deleted PRs are not discoverable from this inventory; consumption is unknown.",
            "snapshot": "Single collection, not an atomic GitHub snapshot; surfaces may change during pagination.",
            "billing": "Included versus paid-credit continuation is unknown; actual runs are not necessarily included allowance consumption.",
            "admission_observability": "Queued/in-progress statuses lack stable run/admission identities; superseded runs can consume allowance without published output. Their charge count/window remains unknown.",
            "count_basis": "Distinct corroborated completed runs are a lower-bound proxy, not total usage; admission timestamps are unavailable. Unknowns are not zero.",
        },
        "errors": [], "not_merge_gate": True,
        "sources": [DOCS + "management/rate-limits",
                    "https://docs.github.com/en/rest/commits/statuses#list-commit-statuses-for-a-reference",
                    "https://docs.github.com/en/rest/pulls/pulls#list-commits-on-a-pull-request"],
    }
    coverage = report["coverage"]
    excluded = report["excluded"]
    ambiguities = report["ambiguities"]
    cache = {}

    def error(operation, message):
        report["errors"].append({"source": "github_reviews", "operation": operation, "message": message})

    def pages(path):
        if path in cache:
            return cache[path]
        rows, seen = [], {}
        capture = {"path": path, "pages": 0, "rows": 0, "complete": False}
        coverage["surfaces"].append(capture)
        page = 1
        while True:
            try:
                payload = run_json(["gh", "api", "--method", "GET", f"repos/{repository}/{path}?per_page=100&page={page}"])
            except RuntimeError:
                # Do not echo arbitrary subprocess output (it can contain credentials).
                error(path, "Read failed, timed out, or returned invalid JSON; this surface is incomplete.")
                break
            if not isinstance(payload, list) or any(not isinstance(row, dict) for row in payload):
                error(path, "Expected a JSON array of objects; this surface is incomplete.")
                break
            capture["pages"] += 1
            new = 0
            for row in payload:
                identity = row.get("id", row.get("sha"))
                if not isinstance(identity, (int, str)):
                    error(path, "A row has no stable source ID; omitted from measurement.")
                    continue
                if identity in seen:
                    if seen[identity] != row:
                        error(path, "A source row changed during pagination; identity is not a stable snapshot.")
                    continue
                seen[identity] = row
                rows.append(row)
                new += 1
            if len(payload) < 100:
                capture["complete"] = True
                break
            if not new:
                error(path, "Pagination repeated a full page without new identities; stopped rather than looping.")
                break
            page += 1
        capture["rows"] = len(rows)
        cache[path] = rows
        return rows

    selected = {}
    for pr in pull_requests:
        number = pr.get("number")
        if not isinstance(number, int) or isinstance(number, bool) or number < 1:
            error("PR inventory", "Invalid PR number; selection is incomplete.")
            continue
        if number in selected:
            if selected[number] != pr:
                error("PR inventory", "Conflicting duplicate PR inventory rows.")
            continue
        updated = _time(pr.get("updatedAt"))
        if pr.get("state") == "OPEN" or updated is None or updated >= window_start:
            selected[number] = pr
        else:
            coverage["omitted_prs"].append({"number": number, "updated_at": pr.get("updatedAt"),
                                             "reason": "Closed/merged PR last updated before candidate window; review usage not queried."})
    coverage["selected_prs"] = sorted(selected)
    events = {}
    status_owners = {}
    for number, pr in sorted(selected.items()):
        commits = pages(f"pulls/{number}/commits")
        if len(commits) >= 250:
            error(f"pulls/{number}/commits", "GitHub REST exposes at most 250 PR commits; additional heads may be omitted.")
        reviews = pages(f"pulls/{number}/reviews")
        comments = pages(f"pulls/{number}/comments")
        issue_comments = pages(f"issues/{number}/comments")
        current_heads = {row["sha"].lower() for row in commits if isinstance(row.get("sha"), str) and SHA.fullmatch(row["sha"])}
        current_head = pr.get("headRefOid")
        if isinstance(current_head, str) and SHA.fullmatch(current_head):
            current_heads.add(current_head.lower())
        heads = set(current_heads)
        run_outputs = {}
        replies = []
        for surface, rows in (("reviews", reviews), ("review_comments", comments), ("issue_comments", issue_comments)):
            for row in rows:
                if not _bot(row):
                    continue
                body = row.get("body") or ""
                if not isinstance(body, str):
                    error(surface, "Provider body was not text; omitted evidence.")
                    continue
                head = row.get("commit_id") or row.get("original_commit_id")
                head = head.lower() if isinstance(head, str) and SHA.fullmatch(head) else None
                if head:
                    heads.add(head)
                if surface == "issue_comments":
                    for marker in COVERAGE.findall(body):
                        try:
                            covered = json.loads(marker)
                        except ValueError:
                            continue
                        if isinstance(covered, dict):
                            for field in ("sourceCommitId", "coveredCommitId"):
                                value = covered.get(field)
                                if isinstance(value, str) and SHA.fullmatch(value):
                                    heads.add(value.lower())
                evidence = _evidence(surface, row, head)
                diagnostic = _diagnostic(body)
                if diagnostic:
                    excluded.append({"pr": number, "reason": diagnostic, "evidence": evidence})
                    # Historical runs on other heads remain valid despite a paused current walkthrough.
                    if surface != "reviews":
                        continue
                ids = RUN.findall(body) if surface == "reviews" and "**Actionable comments posted:" in body else []
                if len(set(ids)) == 1 and head and row.get("state") in ("COMMENTED", "APPROVED", "CHANGES_REQUESTED", "DISMISSED"):
                    run_id = ids[0].lower()
                    submitted = _time(row.get("submitted_at"))
                    if submitted is None or submitted > observed_at:
                        ambiguities.append({"pr": number, "reason": "Run output has missing or after-observation submission time.", "evidence": evidence})
                        continue
                    run_outputs.setdefault(run_id, []).append((head, submitted, evidence))
                    continue
                invocation = INVOCATION.search(body) if surface == "issue_comments" else None
                if invocation and re.search(r"^\s*(?:Full review|Review) finished\.\s*$", body, re.M):
                    replies.append((invocation.group(1), row, evidence))
                elif not diagnostic:
                    excluded.append({"pr": number, "reason": "Mutable walkthrough, empty reply review, finding/reply, or nonreview output; not independently a run.", "evidence": evidence})
        statuses = []
        for head in sorted(heads):
            coverage["heads"].append({"pr": number, "head": head})
            if head not in current_heads:
                coverage["rewritten_heads"].append({"pr": number, "head": head, "source": "surviving provider review/comment reference", "status_history_queried": True})
            for row in pages(f"statuses/{head}"):
                if row.get("context") != "CodeRabbit":
                    continue
                evidence = _evidence("statuses", row, head)
                if not _bot(row, "creator"):
                    ambiguities.append({"pr": number, "reason": "CodeRabbit context without verified provider creator.", "evidence": evidence})
                    continue
                timestamp = _time(row.get("created_at"))
                if timestamp is None:
                    error(f"statuses/{head}", "Provider status has invalid timestamp.")
                    continue
                if timestamp > observed_at:
                    excluded.append({"pr": number, "reason": "Status after observation boundary.", "evidence": evidence})
                elif row.get("description") == "Review completed" and row.get("state") == "success":
                    statuses.append((head, timestamp, evidence))
                else:
                    excluded.append({"pr": number, "reason": "Queued/in-progress/skipped/paused/rate-limited/nonreview status is not a completed run or precise admission.", "description": row.get("description"), "evidence": evidence})
        statuses.sort(key=lambda item: (item[1], str(item[2]["id"])))
        pr_events = {}
        for run_id, outputs in run_outputs.items():
            output_heads = {item[0] for item in outputs}
            if len(output_heads) != 1:
                ambiguities.append({"pr": number, "run_id": run_id, "reason": "Same provider run ID has conflicting heads; not counted."})
                continue
            head, submitted, evidence = min(outputs, key=lambda item: item[1])
            key = "run:" + run_id
            event = {"identity": key, "repository": repository, "pr": number, "head": head,
                     "run_id": run_id, "admitted_at": None, "completed_at": None,
                     "output_submitted_at": _iso(submitted), "evidence": [item[2] for item in outputs],
                     "admission_reason": "Provider run output proves work, not the admission timestamp or included-versus-credit billing.",
                     "completion_basis": "Earliest immutable completed status following this run output before another identified run on the same head; temporal corroboration, not provider accounting.",
                     "aliases": []}
            later_outputs = [item[1] for other, items in run_outputs.items() if other != run_id for item in items if item[0] == head and item[1] > submitted]
            next_output = min(later_outputs) if later_outputs else None
            matches = [item for item in statuses if item[0] == head and item[1] >= submitted and (next_output is None or item[1] < next_output)]
            if matches:
                event["completed_at"] = _iso(matches[0][1])
                event["evidence"].extend(item[2] for item in matches)
                for item in matches:
                    status_owners.setdefault(item[2]["id"], set()).add(key)
                if len(matches) > 1:
                    excluded.append({"pr": number, "reason": "Further completed statuses without a distinct run identity are not additional runs.", "status_ids": [item[2]["id"] for item in matches[1:]]})
            else:
                ambiguities.append({"pr": number, "run_id": run_id, "reason": "Identified run output lacks a corroborating immutable completion timestamp; completion window unknown."})
            pr_events[key] = event
        for invocation, row, evidence in replies:
            start, end = _time(row.get("created_at")), _time(row.get("updated_at"))
            if start is None or end is None or end < start or end > observed_at:
                ambiguities.append({"pr": number, "reason": "Finished command reply has invalid or after-observation edit bounds.", "evidence": evidence})
                continue
            matches = [item for item in statuses if start <= item[1] <= end]
            related = {key for key, event in pr_events.items() if any(start <= item[1] <= end for item in run_outputs.get(event["run_id"], []))}
            if len(related) == 1:
                event = pr_events[next(iter(related))]
                event["aliases"].append(invocation)
                event["evidence"].append(evidence)
                continue
            if related or len({item[0] for item in matches}) != 1 or not matches:
                ambiguities.append({"pr": number, "identity": invocation, "reason": "Finished command reply cannot be uniquely linked to an immutable completion/head; edited timestamp is not a completion.", "evidence": evidence})
                continue
            # A clean full review may have no review object at all. Its stable
            # invocation plus immutable status is evidence; walkthrough edits are not.
            key = "invocation:" + invocation
            head, timestamp, _ = matches[0]
            if key in pr_events:
                pr_events[key]["evidence"].append(evidence)
                continue
            pr_events[key] = {"identity": key, "repository": repository, "pr": number, "head": head,
                              "run_id": None, "admitted_at": None, "completed_at": _iso(timestamp),
                              "admission_reason": "Reply creation is not proof of admission; charge time and billing are unknown.",
                              "completion_basis": "Finished provider invocation corroborated by immutable completed status within reply lifetime; temporal proxy.",
                              "aliases": [], "evidence": [evidence, *[item[2] for item in matches]]}
            for item in matches:
                status_owners.setdefault(item[2]["id"], set()).add(key)
        for key, event in pr_events.items():
            if key in events:
                old = events[key]
                if old["head"] != event["head"] or old["pr"] != number:
                    old["identity_conflict"] = True
                    ambiguities.append({"identity": key, "reason": "Run identity appears on multiple PRs/heads; counted neither twice nor as a known unique completion."})
                old["evidence"].extend(event["evidence"])
            else:
                events[key] = event
        for head, timestamp, evidence in statuses:
            if evidence["id"] not in status_owners:
                ambiguities.append({"pr": number, "head": head, "reason": "Completed status alone is not proof of a distinct actual review; may be paused/spurious/repeated or lack surviving run output.", "evidence": evidence})
    conflicting = set()
    for identity, owners in status_owners.items():
        if len(owners) > 1:
            conflicting.update(owners)
            ambiguities.append({"status_id": identity, "identities": sorted(owners), "reason": "One completion status overlaps multiple candidate runs; no guessed consumption."})
    for key, event in events.items():
        completion = _time(event["completed_at"])
        event["counted_as"] = {"admitted_in_window": None,
                               "completed_in_window": window_start < completion <= observed_at if completion and key not in conflicting and not event.get("identity_conflict") else None}
        report["events"].append(event)
    report["events"].sort(key=lambda event: event["identity"])
    known_completed = sum(event["counted_as"]["completed_in_window"] is True for event in report["events"])
    report["counts"] = {"admitted_in_window": None,
                        "completed_in_window": known_completed if not report["errors"] else None,
                        "admission_time_unknown": len(events),
                        "ambiguous_identities": len(ambiguities)}
    coverage["observed_known_completed_in_window"] = known_completed
    coverage["current_head_gate"] = "Not evaluated: historical usage does not require present-head merge coverage. Ambiguous current-head statuses are not accepted as usage."
    coverage["collection_finished_at"] = _iso(datetime.now(timezone.utc))
    if report["errors"]:
        report["collection_status"] = "partial" if any(item["pages"] for item in coverage["surfaces"]) else "unavailable"
    return report
