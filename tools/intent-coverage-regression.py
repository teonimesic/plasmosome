#!/usr/bin/env python3
"""Exercise intent coverage parsing, derivation, evidence, deadlines, and cleanup.

Requires `bash` and `zsh` on PATH.
"""

import contextlib
import errno
import http.server
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import resource
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

os.environ["PYTHONDONTWRITEBYTECODE"] = "1"
sys.dont_write_bytecode = True

import intent_coverage as coverage


CERTIFICATE = """-----BEGIN CERTIFICATE-----
MIIDHzCCAgegAwIBAgIUdCwNgArdU0BMxyS3QxJ0dVbvGuUwDQYJKoZIhvcNAQEL
BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI2MDkxMjE5MTU1MFoXDTM2MDkw
OTE5MTU1MFowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF
AAOCAQ8AMIIBCgKCAQEA3zELP4Nm7y22si2Rc7X9GYxLhCVavFgbXj87hUlHqyFw
lW8deptC9c3PeIWycD7fdkt2euAo/kDFJlt5+zaDu0Waas3taQ7zlS4aYgq7qwWz
xQ/GlutMOGVsvW43sbYX5WnKyrSZm6pYJB1FWnYnuv3YRxLNE7xBB3C2cx087r1f
mcU+ZSM8KFNJ3mnKSbJUAJLa4K7L2mB4rz5AjIXbPwYZF8OHesJz1ze9uwBYF2Lt
VnGgwNMauUUE4kXhB2RgoJd4BhSU67Stl47EvXv2ylS+1tw3ZG8fN8N1qE5T2J+9
TElYA5/Zkw8SUlqM8SNIvR4DgaAMqbPglk1qE8JSwQIDAQABo2kwZzAdBgNVHQ4E
FgQUN+2WeVPdAd2Z1YpzT6huZDybm58wHwYDVR0jBBgwFoAUN+2WeVPdAd2Z1Ypz
T6huZDybm58wDwYDVR0TAQH/BAUwAwEB/zAUBgNVHREEDTALgglsb2NhbGhvc3Qw
DQYJKoZIhvcNAQELBQADggEBAF6CbXfqO8SmiOmjb0RBazXgbbIdsKU3hGuC3drC
kMrBGT+zkq9fWsVrl8RXNb79sUw6fKWH0b3gO+nKNV+saWJPsiQpDsqXEJnWKrDR
mk8+m7g16zW/3DYkfBDO1ABbEH81g9dWNCXnSqHzqIW1ayhbjxORU9SOLbx/U5jz
xmV7N/O1gcbYgmsBCC6JPgCRj6bckVmJ5gmCsFZUoqBsia7RqR75Ql+2/BPqDqHa
orkbyd+aPa8W593NFd+2lhySkF40q1iWo20irh3fAm/sIA+glvcahfUkah51lJOd
aBihF4A9ZZ6HswstPrg+HErsMP0Rr6emW7IrcGDCrYLiBOU=
-----END CERTIFICATE-----
"""
PRIVATE_KEY = """-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDfMQs/g2bvLbay
LZFztf0ZjEuEJVq8WBtePzuFSUerIXCVbx16m0L1zc94hbJwPt92S3Z64Cj+QMUm
W3n7NoO7RZpqze1pDvOVLhpiCrurBbPFD8aW60w4ZWy9bjexthflacrKtJmbqlgk
HUVadie6/dhHEs0TvEEHcLZzHTzuvV+ZxT5lIzwoU0neacpJslQAktrgrsvaYHiv
PkCMhds/BhkXw4d6wnPXN727AFgXYu1WcaDA0xq5RQTiReEHZGCgl3gGFJTrtK2X
jsS9e/bKVL7W3Ddkbx83w3WoTlPYn71MSVgDn9mTDxJSWozxI0i9HgOBoAyps+CW
TWoTwlLBAgMBAAECggEABrtjODd1rZ2unsm2kyuoVnKvnW0GA1vR+YIFQNQww/Xm
gC9hj37q4mCikyNPOkj/MwF8MaEcw2x/NdS5BW2DxCyJh5KuCEbx7Gw/Ufk6Z2Js
5XMAHbG7sxDGGqGeLu9L4bjS69HON06ISZCTdLLPKHn7IcusoV2ChZ+t9SNq4NWJ
hs3cmyrgnx3RmVGNzZvpTaZ1qjjxG3Dm3nbNE7symXt6tht9SZeqiI/GWpbxhTh1
8japCRGC7yTOFLnPFLbxYq8uXYVP1NFmYEK1fB7nLLxwWlcALRUfEfvSknskEsNp
vCXosY+ttyiuf4SalOI6K9HksLnHlMxgwEcSqzmkQQKBgQDxAo9UzCqjQ0zgyQQe
njQC8iN+WgTzQE3Iq2Gw7HtU6zI+CrAXThHfPSp/AzJmEyiDtzZhki8rudAwNoDs
x4DC8qgxlKNC1tWGaIhKpdZLXDW4PlCaOENw2mTIM7CVive7H+2M8IsDdnSsHVtM
OkHjGw8ipxV2/+RZUszd8jvbsQKBgQDtEsX3c2qndvxH+iYZmVW6XmcpaZunqff4
Ine0C12Xj41ABZIsRU1EDV5saynZAC7+mHNzy+sxk3M/YSf0Jj5zef6M5n4g4o+t
IufRLKyHDbCLyd+g6tyAR4/h5fh8wOr+1KHgUTpsAMXT+C5Y90T6yO8NEHv5uoWr
5MJgIA98EQKBgDKLBuQRrR9wDb9WaLbDFsVHYoos9rzMz4M17dbcwUCd0nuQYj2A
8d6PRUo9sWQWwHhfA9iSf7H71d1GkOMXM7muifdb5KEvzLfTVEHTZY2IWPu6lczB
3+La6ifSL0YtTqa/m2HjUEP5o540yeDClu65zgLGZ4n9QDY7Vxt0oXkBAoGAVODr
z/Sasup+2KZPDctATkGOXd1ZxWWtSkHM6cFH+QOEZu+XrhIB3+OJcvfLO849BRo/
+61+v3kzQfXfACLRKTb8VCYR8mQrXKmqpdGA07mrA+F7F3n/CE6WzSIxHTlU6Xfn
nRB4AkMkkQfCUEf3gnJ+ZAcK3BZT1X9JuHDCGoECgYEAqJB8oRm8VgBQRt363zTw
ije+cFOg5WFNRSuvUOgx45zsRcu5zot2YB6P92Jqo8nkajvbR7sfLWvNLCAr4m8h
6BdLrqvGFrPdwEV4cpolXYqIUDnp78dY5VWEHagplqBb7/Jl7aArkv33O36jSchm
fjCUh0LFJ081lZH1UpsNl3w=
-----END PRIVATE KEY-----
"""
MERGED_AT = "2026-09-12T12:34:56Z"
COMMIT = "a" * 40
PR_URL = "https://github.com/teonimesic/plasmosome/pull/6"
SCALED_LIMITS = coverage.Limits(0.2, 0.6, 1.2, 6.0)
REQUIRED_SHELLS = ("bash", "zsh")


class FixtureNativeReader:
    def __init__(self, rows):
        self.rows = rows
        self.calls = 0

    def read(self, deadline):
        self.calls += 1
        return self.rows


class FixtureForgeReader:
    def __init__(self, observations=None, refusal=None):
        self.observations = observations or {}
        self.refusal = refusal
        self.calls = []

    def observe(self, url, deadline):
        self.calls.append(url)
        if self.refusal is not None:
            raise coverage.Refusal(url, self.refusal)
        if url not in self.observations:
            raise coverage.Refusal(url, "fixture observation unavailable")
        return self.observations[url]


class FixtureTokenReader:
    def read(self, deadline):
        return "fixture-token"


def intent_text(identity="001", status="approved", served="partly", body="Goal"):
    served_line = "" if served is None else f"served: {served}\n"
    return f"---\nid: {identity}\nstatus: {status}\n{served_line}outcome:\n---\n\n{body}\n"


def spec_text(identity="001", status="accepted", intents=("001",)):
    values = ", ".join(intents)
    return f"---\nid: {identity}\nstatus: {status}\nintents: [{values}]\n---\n\nBehavior\n"


def native_task(
    identity="task-alpha",
    status="open",
    spec_ids=("001",),
    intent_ids=("001",),
    closure=None,
    closed_at=None,
    external_ref=None,
):
    metadata = {"spec_ids": list(spec_ids), "intent_ids": list(intent_ids)}
    if closure is not None:
        metadata["closure"] = closure
    row = {"id": identity, "status": status, "metadata": metadata}
    if closed_at is not None:
        row["closed_at"] = closed_at
    if external_ref is not None:
        row["external_ref"] = external_ref
    return row


def native_carrier(
    identity="carrier-alpha",
    status="open",
    intent_ids=("001",),
    close_reason=None,
):
    row = {
        "id": identity,
        "status": status,
        "issue_type": "chore",
        "labels": ["planner-dispatch"],
        "metadata": {"intent_ids": list(intent_ids)},
    }
    if close_reason is not None:
        row["close_reason"] = close_reason
    return row


def delivered_task(identity="task-delivered", url=PR_URL):
    return native_task(
        identity,
        "closed",
        closure={"kind": "delivered", "closed_at": MERGED_AT},
        closed_at=MERGED_AT,
        external_ref=url,
    )


def merged_observation(url=PR_URL):
    return {"state": "MERGED", "merge_commit": COMMIT, "merged_at": MERGED_AT, "url": url}


def filesystem_inventory(root):
    inventory = {}
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if path.is_dir():
            inventory[relative] = ("directory",)
        elif path.is_file():
            inventory[relative] = ("file", path.read_bytes())
        else:
            inventory[relative] = ("other",)
    return inventory


def wait_for_pid(path, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            return int(path.read_text())
        except (FileNotFoundError, ValueError):
            time.sleep(0.01)
    raise AssertionError(f"helper did not publish its PID at {path}")


class ManualClock:
    def __init__(self):
        self.value = 0.0

    def __call__(self):
        return self.value

    def sleep(self, duration):
        self.value += duration


def delayed_worker_start(delay, target, arguments):
    time.sleep(delay)
    target(*arguments)


class DelayedStartContext:
    def __init__(self, delay):
        self.context = multiprocessing.get_context("spawn")
        self.delay = delay

    def Pipe(self, duplex=True):
        return self.context.Pipe(duplex)

    def Process(self, target, args):
        return self.context.Process(
            target=delayed_worker_start,
            args=(self.delay, target, args),
        )


class FixtureRoot:
    def __init__(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="intent-coverage-regression-")
        self.root = Path(self.temporary.name)
        (self.root / "docs/intents").mkdir(parents=True)
        (self.root / "docs/specs").mkdir(parents=True)
        self.write_intent()
        self.write_spec()

    def write_intent(self, text=None, name="001-goal.md"):
        (self.root / "docs/intents" / name).write_text(text or intent_text(), encoding="utf-8")

    def write_spec(self, text=None, name="001-behavior.md"):
        (self.root / "docs/specs" / name).write_text(text or spec_text(), encoding="utf-8")

    def close(self):
        self.temporary.cleanup()


class CoverageBehaviorTests(unittest.TestCase):
    def setUp(self):
        self.fixture = FixtureRoot()

    def tearDown(self):
        self.fixture.close()

    def execute(self, rows=(), observations=None, command="check", requested_id=None):
        native = FixtureNativeReader(list(rows))
        forge = FixtureForgeReader(observations)
        result = coverage.run(command, requested_id, self.fixture.root, native, forge)
        return result, native, forge

    def test_clean_check_and_three_faults(self):
        result, unused, unused_forge = self.execute()
        self.assertEqual(result, coverage.RunResult(0, (), ()))
        self.fixture.write_intent(intent_text(served="substantially"))
        result, unused, unused_forge = self.execute()
        self.assertEqual(result.exit_code, 1)
        self.assertEqual(result.stdout_lines, ("docs/intents/001-goal.md: fault2",))
        self.fixture.write_intent(intent_text(served="none"))
        result, unused, unused_forge = self.execute(
            [delivered_task()],
            {PR_URL: merged_observation()},
        )
        self.assertEqual(result.exit_code, 1)
        self.assertEqual(result.stdout_lines, ("docs/intents/001-goal.md: fault1",))
        self.fixture.write_intent(intent_text(served="partly"))
        result, unused, unused_forge = self.execute(
            [delivered_task()],
            {PR_URL: merged_observation()},
        )
        self.assertEqual(result, coverage.RunResult(0, (), ()))

    def test_every_malformed_served_shape_is_fault_three_once(self):
        forms = {
            "absent": intent_text(served=None),
            "empty": intent_text(served=""),
            "invalid": intent_text(served="mostly"),
            "misplaced": intent_text().replace("status: approved\nserved: partly", "served: partly\nstatus: approved"),
            "body-only": intent_text(served=None, body="served: partly"),
            "duplicate": intent_text(body="served: mostly"),
        }
        for name, text in forms.items():
            with self.subTest(name=name):
                self.fixture.write_intent(text)
                result, unused, unused_forge = self.execute()
                self.assertEqual(result.exit_code, 1)
                self.assertEqual(result.stdout_lines, ("docs/intents/001-goal.md: fault3",))

    def test_input_fault_precedes_malformed_coverage(self):
        self.fixture.write_intent(intent_text(served="mostly"))
        self.fixture.write_spec(spec_text(intents=("099",)))
        result, native, forge = self.execute()
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())
        self.assertIn("missing intent 099", result.stderr_lines[0])
        self.assertEqual(native.calls, 0)
        self.assertEqual(forge.calls, [])

    def test_show_sorts_specs_full_ids_and_deduplicates_two_paths(self):
        self.fixture.write_spec(spec_text("002"), "002-other.md")
        rows = [
            native_task("plasmosome-z", "review", ("002", "001"), ("001",)),
            native_task("plasmosome-a", "planning", (), ("001",)),
            native_task("plasmosome-b", "in_progress"),
            native_task("plasmosome-c", "blocked"),
            native_task("plasmosome-d", "open"),
            native_task(
                "plasmosome-collision-a",
                "closed",
                closure={"kind": "cancelled", "closed_at": MERGED_AT, "reason": "Superseded explicitly"},
                closed_at=MERGED_AT,
            ),
        ]
        result, unused, unused_forge = self.execute(rows, command="show", requested_id="001")
        self.assertEqual(result.exit_code, 0)
        decoded = [json.loads(line) for line in result.stdout_lines]
        self.assertEqual(decoded[0], {"type": "spec", "intent_id": "001", "id": "001", "status": "accepted"})
        spec_rows = [row for row in decoded if row["type"] == "spec"]
        self.assertEqual(spec_rows, [
            {"type": "spec", "intent_id": "001", "id": "001", "status": "accepted"},
            {"type": "spec", "intent_id": "001", "id": "002", "status": "accepted"},
        ])
        task_rows = [row for row in decoded if row["type"] == "task"]
        self.assertEqual(len(task_rows), len(rows))
        self.assertEqual(len({row["id"] for row in task_rows}), len(rows))
        self.assertEqual(next(row for row in task_rows if row["id"] == "plasmosome-z")["spec_id"], "001")
        self.assertEqual(next(row for row in task_rows if row["id"] == "plasmosome-a")["spec_id"], None)
        self.assertEqual(
            {row["status"] for row in task_rows},
            {"open", "planning", "in_progress", "review", "blocked", "closed"},
        )
        cancelled = next(row for row in task_rows if row["id"] == "plasmosome-collision-a")
        self.assertEqual(cancelled["reason"], "Superseded explicitly")

    def test_show_taskless_spec_and_empty_existing_intent(self):
        result, unused, unused_forge = self.execute(command="show", requested_id="001")
        self.assertEqual(len(result.stdout_lines), 1)
        self.assertEqual(json.loads(result.stdout_lines[0])["type"], "spec")
        (self.fixture.root / "docs/specs/001-behavior.md").write_text(spec_text(intents=("002",)))
        self.fixture.write_intent(intent_text("002"), "002-other.md")
        result, unused, unused_forge = self.execute(command="show", requested_id="001")
        self.assertEqual(result, coverage.RunResult(0, (), ()))

    def test_show_unknown_or_non_numeric_intent_refuses(self):
        for requested in ("999", "1", "abc"):
            with self.subTest(requested=requested):
                result, unused, unused_forge = self.execute(command="show", requested_id=requested)
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())

    def test_document_authority_boundaries_refuse(self):
        cases = []
        cases.append(("missing-intents", lambda: shutil.rmtree(self.fixture.root / "docs/intents")))
        cases.append(("empty-intents", lambda: (self.fixture.root / "docs/intents/001-goal.md").unlink()))
        cases.append(("readme-only", lambda: ((self.fixture.root / "docs/intents/001-goal.md").unlink(), (self.fixture.root / "docs/intents/README.md").write_text("index"))))
        cases.append(("bad-id", lambda: self.fixture.write_intent(intent_text("1"))))
        cases.append(("bad-state", lambda: self.fixture.write_intent(intent_text(status="settled"))))
        cases.append(("duplicate-id", lambda: self.fixture.write_intent(intent_text(), "002-duplicate.md")))
        cases.append(("accepted-draft", lambda: self.fixture.write_intent(intent_text(status="draft"))))
        for name, mutation in cases:
            with self.subTest(name=name):
                self.fixture.close()
                self.fixture = FixtureRoot()
                mutation()
                result, unused, unused_forge = self.execute()
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())
                self.assertTrue(result.stderr_lines[0].startswith("input:"))

    def test_document_directory_enumeration_failure_is_input_refusal(self):
        directory = self.fixture.root / "docs/intents"
        original_iterdir = Path.iterdir

        def unavailable(selected):
            if selected == directory:
                raise PermissionError(errno.EACCES, "fixture enumeration unavailable")
            return original_iterdir(selected)

        with mock.patch.object(Path, "iterdir", unavailable):
            result, native, forge = self.execute()
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())
        self.assertEqual(len(result.stderr_lines), 1)
        self.assertTrue(result.stderr_lines[0].startswith("input: docs/intents:"))
        self.assertEqual(native.calls, 0)
        self.assertEqual(forge.calls, [])

    def test_numeric_document_metadata_failure_is_input_refusal(self):
        path = self.fixture.root / "docs/intents/001-goal.md"
        original_lstat = Path.lstat

        def unavailable(selected):
            if selected == path:
                raise PermissionError(errno.EACCES, "fixture metadata unavailable")
            return original_lstat(selected)

        with mock.patch.object(Path, "lstat", unavailable):
            result, native, forge = self.execute()
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())
        self.assertEqual(len(result.stderr_lines), 1)
        self.assertTrue(result.stderr_lines[0].startswith("input: docs/intents/001-goal.md:"))
        self.assertEqual(native.calls, 0)
        self.assertEqual(forge.calls, [])

    def test_native_structural_boundaries_and_all_six_states(self):
        valid = [
            native_task(f"task-{state}", state)
            for state in ("open", "planning", "in_progress", "review", "blocked")
        ]
        valid.append(native_task("task-closed", "closed", closure={"kind": "cancelled", "closed_at": MERGED_AT, "reason": "Not delivered"}, closed_at=MERGED_AT))
        result, unused, unused_forge = self.execute(valid)
        self.assertEqual(result.exit_code, 0)
        malformed = [
            [{"id": "x", "status": "planned", "metadata": {"spec_ids": [], "intent_ids": []}}],
            [{"id": "x", "status": "open", "metadata": {"spec_ids": "001", "intent_ids": []}}],
            [{"id": "x", "status": "open", "metadata": {"spec_ids": ["999"], "intent_ids": ["001"]}}],
            [{"id": "x", "status": "open", "metadata": {"spec_ids": ["001"], "intent_ids": []}}],
            [native_task("same"), native_task("same")],
        ]
        for rows in malformed:
            with self.subTest(rows=rows):
                result, unused, unused_forge = self.execute(rows)
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())

    def test_valid_standalone_carriers_are_excluded_from_coverage(self):
        self.fixture.write_intent(intent_text("002"), "002-other.md")
        null_assignee = native_carrier("null-assignee")
        null_assignee["assignee"] = None
        empty_assignee = native_carrier("empty-assignee")
        empty_assignee["assignee"] = ""
        rows = [
            native_carrier("open-carrier"),
            native_carrier("accepted-spec-carrier", "closed", close_reason="Accepted spec reached main"),
            native_carrier("cancelled-carrier", "closed", close_reason="Intent no longer wants a spec"),
            native_carrier("duplicate-carrier", "closed", close_reason="Duplicate of surviving carrier"),
            native_carrier("other-subject-carrier", intent_ids=("002",)),
            null_assignee,
            empty_assignee,
        ]
        result, unused, forge = self.execute(rows, command="show", requested_id="001")
        self.assertEqual(result.exit_code, 0)
        self.assertEqual(
            [json.loads(line) for line in result.stdout_lines],
            [{"type": "spec", "intent_id": "001", "id": "001", "status": "accepted"}],
        )
        self.assertEqual(forge.calls, [])
        ordinary_chore = native_task("ordinary-chore")
        ordinary_chore["issue_type"] = "chore"
        task_dispatch_notes = native_task("task-dispatch-notes")
        task_dispatch_notes["notes"] = "planner dispatch entry"
        result, unused, forge = self.execute(
            [ordinary_chore, task_dispatch_notes],
            command="show",
            requested_id="001",
        )
        task_ids = [json.loads(line)["id"] for line in result.stdout_lines if json.loads(line)["type"] == "task"]
        self.assertEqual(task_ids, ["ordinary-chore", "task-dispatch-notes"])
        self.assertEqual(forge.calls, [])

    def test_closed_carrier_may_retain_a_now_draft_intent(self):
        self.fixture.write_intent(intent_text("002", status="draft"), "002-other.md")
        closed = native_carrier("closed-carrier", "closed", ("002",), "Intent no longer wants a spec")
        result, unused, forge = self.execute([closed])
        self.assertEqual(result, coverage.RunResult(0, (), ()))
        self.assertEqual(forge.calls, [])
        result, unused, forge = self.execute([native_carrier("open-carrier", intent_ids=("002",))])
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())
        self.assertEqual(forge.calls, [])

    def test_malformed_carrier_and_near_marker_shapes_refuse(self):
        non_chore = native_carrier("non-chore")
        non_chore["issue_type"] = "task"
        ordinary_chore = native_carrier("ordinary-chore")
        ordinary_chore["labels"] = []
        task_dispatch_notes = {
            "id": "task-dispatch-notes",
            "status": "open",
            "notes": "planner dispatch entry",
            "metadata": {"intent_ids": ["001"]},
        }
        labels_wrong_type = native_task("labels-wrong-type")
        labels_wrong_type["labels"] = "planner-dispatch"
        labels_null = native_task("labels-null")
        labels_null["labels"] = None
        label_member_wrong_type = native_task("label-member-wrong-type")
        label_member_wrong_type["labels"] = [1]
        metadata_wrong_type = native_carrier("metadata-wrong-type")
        metadata_wrong_type["metadata"] = "metadata"
        missing_intent = native_carrier("missing-intent")
        missing_intent["metadata"] = {}
        multiple_intents = native_carrier("multiple-intents", intent_ids=("001", "002"))
        dangling_intent = native_carrier("dangling-intent", intent_ids=("999",))
        empty_intents = native_carrier("empty-intents", intent_ids=())
        malformed_intent = native_carrier("malformed-intent", intent_ids=("1",))
        spec_ids_null = native_carrier("spec-ids-null")
        spec_ids_null["metadata"]["spec_ids"] = None
        spec_ids_empty = native_carrier("spec-ids-empty")
        spec_ids_empty["metadata"]["spec_ids"] = []
        planned = native_carrier("planned")
        planned["labels"].append("planned")
        needs_plan = native_carrier("needs-plan")
        needs_plan["labels"].append("needs-plan")
        task_phase = native_carrier("task-phase", status="planning")
        assigned_space = native_carrier("assigned-space")
        assigned_space["assignee"] = " "
        assigned_tab = native_carrier("assigned-tab")
        assigned_tab["assignee"] = "\t"
        assigned_wrong_type = native_carrier("assigned-wrong-type")
        assigned_wrong_type["assignee"] = 0
        missing_reason = native_carrier("missing-reason", status="closed")
        blank_reason = native_carrier("blank-reason", status="closed", close_reason=" ")
        cases = [
            non_chore,
            ordinary_chore,
            task_dispatch_notes,
            labels_wrong_type,
            label_member_wrong_type,
            metadata_wrong_type,
            missing_intent,
            multiple_intents,
            labels_null,
            dangling_intent,
            spec_ids_null,
            spec_ids_empty,
            planned,
            empty_intents,
            malformed_intent,
            needs_plan,
            task_phase,
            assigned_space,
            assigned_tab,
            assigned_wrong_type,
            missing_reason,
            blank_reason,
        ]
        for row in cases:
            with self.subTest(identity=row["id"]):
                result, unused, forge = self.execute([row])
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())
                self.assertEqual(forge.calls, [])

    def test_carrier_duplicate_identity_is_global_but_subject_is_not_identity(self):
        carrier = native_carrier("same")
        task = native_task("same")
        for rows in ([carrier, task], [task, carrier], [carrier, native_carrier("same")]):
            with self.subTest(order=[row["issue_type"] if "issue_type" in row else "task" for row in rows]):
                result, unused, unused_forge = self.execute(rows)
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())
        result, unused, forge = self.execute(
            [native_carrier("first"), native_carrier("second")],
            command="show",
            requested_id="001",
        )
        self.assertEqual(result.exit_code, 0)
        self.assertEqual(len(result.stdout_lines), 1)
        self.assertEqual(forge.calls, [])

    def test_carrier_cannot_mask_ordinary_input_fault_precedence(self):
        self.fixture.write_intent(intent_text(served="mostly"))
        bad_link = native_task("bad-link", spec_ids=("999",))
        result, unused, forge = self.execute([native_carrier(), bad_link])
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())
        self.assertTrue(result.stderr_lines[0].startswith("input: bad-link:"))
        self.assertEqual(forge.calls, [])
        unknown_closure = native_task("unknown-closure", status="closed", closed_at=MERGED_AT)
        result, unused, forge = self.execute([native_carrier(), unknown_closure])
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())
        self.assertTrue(result.stderr_lines[0].startswith("input: unknown-closure:"))
        self.assertEqual(forge.calls, [])

    def test_literal_native_ids_remain_distinct(self):
        rows = [native_task("plasmosome-043-a", "open"), native_task("plasmosome-043-b", "open")]
        result, unused, unused_forge = self.execute(rows, command="show", requested_id="001")
        decoded = [json.loads(line) for line in result.stdout_lines]
        self.assertEqual([row["id"] for row in decoded if row["type"] == "task"], ["plasmosome-043-a", "plasmosome-043-b"])

    def test_closure_annotation_validation_is_exact(self):
        invalid = [
            ("absent", None, None, None),
            ("wrong-annotation-type", "delivered", None, None),
            ("empty-object", {}, None, None),
            ("array-kind", {"kind": [], "closed_at": MERGED_AT}, None, None),
            ("object-kind", {"kind": {}, "closed_at": MERGED_AT}, None, None),
            ("unknown-kind", {"kind": "other", "closed_at": MERGED_AT}, None, None),
            (
                "extra-delivered-key",
                {"kind": "delivered", "closed_at": MERGED_AT, "extra": True},
                PR_URL,
                {PR_URL: merged_observation()},
            ),
            (
                "stale-cancellation-binding",
                {"kind": "cancelled", "closed_at": "2026-09-12T00:00:00Z", "reason": "Withdrawn"},
                None,
                None,
            ),
            ("cancelled-missing-reason", {"kind": "cancelled", "closed_at": MERGED_AT}, None, None),
            (
                "cancelled-blank-reason",
                {"kind": "cancelled", "closed_at": MERGED_AT, "reason": "   "},
                None,
                None,
            ),
            (
                "cancelled-generic-reason",
                {"kind": "cancelled", "closed_at": MERGED_AT, "reason": " Closed "},
                None,
                None,
            ),
        ]
        for name, closure, external_ref, observations in invalid:
            with self.subTest(name=name):
                row = native_task(
                    "closed-row",
                    "closed",
                    closure=closure,
                    closed_at=MERGED_AT,
                    external_ref=external_ref,
                )
                result, unused, unused_forge = self.execute([row], observations)
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())
                self.assertTrue(result.stderr_lines[0].startswith("input: closed-row:"), result.stderr_lines)
        row = native_task("closed-row", "closed", closure={"kind": "cancelled", "closed_at": MERGED_AT, "reason": "Explicit\nreason"}, closed_at=MERGED_AT)
        result, unused, unused_forge = self.execute([row], command="show", requested_id="001")
        self.assertEqual(result.exit_code, 0)
        self.assertEqual(len(result.stdout_lines), 2)
        self.assertNotIn("\n", result.stdout_lines[1])

    def test_rfc3339_component_boundaries_and_exact_cancellation_binding(self):
        valid = (
            MERGED_AT,
            "2026-09-12T12:34:56.123456+01:00",
            "2016-12-31T23:59:60Z",
            "2017-01-01T00:59:60+01:00",
            "2026-09-12t12:34:56.123456z",
            "2026-09-12T12:34:56z",
            "2026-09-12t12:34:56Z",
            "2016-12-31t23:59:60z",
            "2017-01-01t00:59:60+01:00",
        )
        invalid = (
            "2026-09-12T12:00:00+00:60",
            "2026-09-12T12:00:00+01:99",
            "2026-09-12T12:00:00+24:00",
            "2026-09-12T12:00:61Z",
            "2026-09-12T23:59:60Z",
            "2026-02-30T12:00:00Z",
        )
        for stamp in valid:
            with self.subTest(stamp=stamp):
                self.assertTrue(coverage.valid_rfc3339(stamp))
                row = native_task(
                    "valid-cancellation",
                    "closed",
                    closure={"kind": "cancelled", "closed_at": stamp, "reason": "Withdrawn"},
                    closed_at=stamp,
                )
                result, unused, unused_forge = self.execute([row])
                self.assertEqual(result, coverage.RunResult(0, (), ()))
        for stamp in invalid:
            with self.subTest(stamp=stamp):
                self.assertFalse(coverage.valid_rfc3339(stamp))
                row = native_task(
                    "invalid-timestamp",
                    "closed",
                    closure={"kind": "cancelled", "closed_at": stamp, "reason": "Withdrawn"},
                    closed_at=stamp,
                )
                result, unused, unused_forge = self.execute([row])
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())
                self.assertTrue(result.stderr_lines[0].startswith("input: invalid-timestamp:"), result.stderr_lines)
        row = native_task(
            "mixed-case-binding",
            "closed",
            closure={"kind": "cancelled", "closed_at": "2026-09-12T12:00:00Z", "reason": "Withdrawn"},
            closed_at="2026-09-12t12:00:00z",
        )
        result, unused, unused_forge = self.execute([row])
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())

    def test_nonclosed_stale_closure_and_literal_assignee_do_not_deliver(self):
        row = native_task("reopened", "review")
        row["metadata"]["closure"] = {"kind": "delivered", "closed_at": MERGED_AT}
        row["assignee"] = "   "
        row["external_ref"] = PR_URL
        result, unused, forge = self.execute([row], command="show", requested_id="001")
        task_row = json.loads(result.stdout_lines[1])
        self.assertEqual(task_row["status"], "review")
        self.assertEqual(task_row["disposition"], "open")
        self.assertEqual(forge.calls, [])

    def test_delivered_and_cancelled_forge_evidence(self):
        result, unused, forge = self.execute([delivered_task()], {PR_URL: merged_observation()}, "show", "001")
        row = json.loads(result.stdout_lines[1])
        self.assertEqual((row["pr_url"], row["merge_commit"], row["merged_at"]), (PR_URL, COMMIT, MERGED_AT))
        self.assertEqual(forge.calls, [PR_URL])
        cancelled = native_task(
            "cancelled",
            "closed",
            closure={"kind": "cancelled", "closed_at": MERGED_AT, "reason": "Proposal withdrawn"},
            closed_at=MERGED_AT,
            external_ref=PR_URL,
        )
        closed = {"state": "CLOSED", "merge_commit": None, "merged_at": None, "url": PR_URL}
        result, unused, forge = self.execute([cancelled], {PR_URL: closed}, "show", "001")
        row = json.loads(result.stdout_lines[1])
        self.assertEqual((row["disposition"], row["reason"], row["pr_url"]), ("cancelled", "Proposal withdrawn", PR_URL))

    def test_conflicting_or_unknown_forge_evidence_refuses_without_output(self):
        cases = [
            {"state": "CLOSED", "merge_commit": None, "merged_at": None, "url": PR_URL},
            {"state": "MERGED", "merge_commit": "short", "merged_at": MERGED_AT, "url": PR_URL},
            {"state": "MERGED", "merge_commit": COMMIT, "merged_at": None, "url": PR_URL},
        ]
        for observation in cases:
            with self.subTest(observation=observation):
                result, unused, unused_forge = self.execute([delivered_task()], {PR_URL: observation}, "show", "001")
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())
        result, unused, unused_forge = self.execute([delivered_task()], command="show", requested_id="001")
        self.assertEqual(result.exit_code, 2)
        self.assertEqual(result.stdout_lines, ())

    def test_repeated_pull_request_is_observed_once(self):
        rows = [delivered_task("one"), delivered_task("two")]
        result, unused, forge = self.execute(rows, {PR_URL: merged_observation()}, "show", "001")
        self.assertEqual(result.exit_code, 0)
        self.assertEqual(forge.calls, [PR_URL])


class NativeAdapterTests(unittest.TestCase):
    def test_exact_unbounded_invocation_and_json_contract(self):
        with tempfile.TemporaryDirectory(prefix="intent-native-adapter-") as temporary:
            root = Path(temporary)
            (root / "tools").mkdir()
            helper = root / "tools/work-state"
            helper.write_text(
                "#!/usr/bin/env python3\n"
                "import json, sys\n"
                "assert sys.argv[1:] == ['list','--all','--limit','0','--json'], sys.argv\n"
                "print(json.dumps([]))\n"
            )
            helper.chmod(0o700)
            reader = coverage.NativeReader(root)
            self.assertEqual(reader.read(time.monotonic() + 2), [])
            helper.write_text("#!/usr/bin/env python3\nprint('{')\n")
            helper.chmod(0o700)
            with self.assertRaises(coverage.Refusal):
                reader.read(time.monotonic() + 2)

    def test_group_probe_permission_then_absence_completes_before_deadline(self):
        clock = ManualClock()
        outcomes = iter((PermissionError(), ProcessLookupError()))

        def group_probe(unused_group, unused_signal):
            raise next(outcomes)

        with (
            mock.patch.object(coverage.os, "waitpid", side_effect=ChildProcessError()),
            mock.patch.object(coverage.os, "killpg", side_effect=group_probe),
            mock.patch.object(coverage.time, "sleep", side_effect=clock.sleep),
        ):
            completed = coverage._finish_waitable_process_group(200, 0.05, clock)

        self.assertTrue(completed)
        self.assertLess(clock(), 0.05)

    def test_persistent_group_probe_permission_refuses_at_deadline(self):
        with tempfile.TemporaryDirectory(prefix="intent-helper-permission-") as temporary:
            root = Path(temporary)
            pid_path = root / "child.pid"
            helper = root / "helper.py"
            helper.write_text(
                "import pathlib, subprocess, sys\n"
                "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'])\n"
                "pathlib.Path(sys.argv[1]).write_text(str(child.pid))\n"
            )
            spawned = []
            cleanup_failures = []
            product_group_absent = False
            original_popen = coverage.subprocess.Popen
            original_killpg = coverage.os.killpg

            def recording_popen(*arguments, **keywords):
                process = original_popen(*arguments, **keywords)
                spawned.append(process)
                return process

            def persistent_permission(process_group, selected_signal):
                if selected_signal == 0:
                    raise PermissionError()
                return original_killpg(process_group, selected_signal)

            try:
                started = time.monotonic()
                deadline = started + 0.6
                with (
                    mock.patch.object(coverage.subprocess, "Popen", side_effect=recording_popen),
                    mock.patch.object(coverage.os, "killpg", side_effect=persistent_permission),
                ):
                    with self.assertRaises(coverage.Refusal) as caught:
                        coverage.run_owned_process(
                            [sys.executable, str(helper), str(pid_path)],
                            root,
                            deadline,
                            time.monotonic,
                            "fixture helper",
                        )
                finished = time.monotonic()

                self.assertEqual(
                    caught.exception.reason,
                    "owned helper group cleanup did not complete within the command deadline",
                )
                self.assertGreaterEqual(finished, deadline)
                self.assertLess(finished - started, 0.8)
                child_pid = int(pid_path.read_text())
                with self.assertRaises(ProcessLookupError):
                    os.kill(child_pid, 0)
                with self.assertRaises(ProcessLookupError):
                    os.killpg(spawned[0].pid, 0)
                product_group_absent = True
                self.assertIsNotNone(spawned[0].poll())
            finally:
                teardown_deadline = time.monotonic() + 1
                for process in spawned:
                    if not product_group_absent:
                        try:
                            original_killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            product_group_absent = True
                        except BaseException as error:
                            cleanup_failures.append(error)
                    if process.poll() is None:
                        try:
                            process.kill()
                        except ProcessLookupError:
                            pass
                        except BaseException as error:
                            cleanup_failures.append(error)
                    try:
                        process.wait(timeout=max(0.0, teardown_deadline - time.monotonic()))
                    except BaseException as error:
                        cleanup_failures.append(error)
                    if product_group_absent:
                        continue
                    while True:
                        try:
                            original_killpg(process.pid, 0)
                        except ProcessLookupError:
                            break
                        except PermissionError:
                            pass
                        except BaseException as error:
                            cleanup_failures.append(error)
                            break
                        remaining = teardown_deadline - time.monotonic()
                        if remaining <= 0:
                            cleanup_failures.append(
                                AssertionError(f"owned fixture group {process.pid} survived independent teardown")
                            )
                            break
                        time.sleep(min(0.01, remaining))
            self.assertEqual(cleanup_failures, [])

    def test_owned_helper_deadline_kills_inherited_pipe_child(self):
        with tempfile.TemporaryDirectory(prefix="intent-helper-cleanup-") as temporary:
            root = Path(temporary)
            pid_path = root / "child.pid"
            helper = root / "helper.py"
            helper.write_text(
                "import os, pathlib, subprocess, sys\n"
                "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'])\n"
                "pathlib.Path(sys.argv[1]).write_text(f'{os.getpid()} {child.pid}')\n"
            )
            unrelated = subprocess.Popen([sys.executable, "-c", "import sys; sys.exit(23)"])
            try:
                waitid = getattr(os, "waitid", None)
                if waitid is not None:
                    unrelated_exit = waitid(os.P_PID, unrelated.pid, os.WEXITED | os.WNOWAIT)
                    self.assertEqual(unrelated_exit.si_status, 23)
                deadline = time.monotonic() + 0.6
                with self.assertRaises(coverage.Refusal) as caught:
                    coverage.run_owned_process(
                        [sys.executable, str(helper), str(pid_path)],
                        root,
                        deadline,
                        time.monotonic,
                        "fixture helper",
                    )
                self.assertEqual(caught.exception.authority, "fixture helper")
                self.assertEqual(caught.exception.reason, "helper deadline exceeded")
                helper_pid, child_pid = map(int, pid_path.read_text().split())
                with self.assertRaises(ProcessLookupError):
                    os.kill(child_pid, 0)
                with self.assertRaises(ProcessLookupError):
                    os.killpg(helper_pid, 0)
                self.assertEqual(unrelated.wait(timeout=1), 23)
            finally:
                if unrelated.poll() is None:
                    unrelated.kill()
                    unrelated.wait()

    def test_native_and_auth_stalls_refuse_and_reap(self):
        with tempfile.TemporaryDirectory(prefix="intent-helper-stalls-") as temporary:
            root = Path(temporary)
            tools = root / "tools"
            binary = root / "bin"
            tools.mkdir()
            binary.mkdir()
            native = tools / "work-state"
            native.write_text("#!/bin/sh\nexec sleep 30\n")
            native.chmod(0o700)
            environment = {
                **os.environ,
                "PATH": f"{binary}:{os.environ['PATH']}",
            }
            environment.pop("GH_TOKEN", None)
            environment.pop("GITHUB_TOKEN", None)
            gh = binary / "gh"
            gh.write_text("#!/bin/sh\nexec sleep 30\n")
            gh.chmod(0o700)

            for name, reader in (
                ("native", coverage.NativeReader(root)),
                ("authentication", coverage.TokenReader(root, environment=environment)),
            ):
                with self.subTest(name=name):
                    spawned = []
                    original_popen = coverage.subprocess.Popen

                    def recording_popen(*arguments, **keywords):
                        process = original_popen(*arguments, **keywords)
                        spawned.append(process.pid)
                        return process

                    started = time.monotonic()
                    with mock.patch.object(coverage.subprocess, "Popen", side_effect=recording_popen):
                        with self.assertRaises(coverage.Refusal):
                            reader.read(started + 0.6)
                    self.assertLess(time.monotonic() - started, 0.7)
                    self.assertEqual(len(spawned), 1)
                    with self.assertRaises(ProcessLookupError):
                        os.kill(spawned[0], 0)

    def test_parent_only_sigterm_reaps_active_native_helper(self):
        source = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory(prefix="intent-parent-signal-") as temporary:
            root = Path(temporary)
            (root / "tools").mkdir()
            (root / "docs/intents").mkdir(parents=True)
            (root / "docs/specs").mkdir(parents=True)
            for name in ("intent-coverage", "intent_coverage.py"):
                shutil.copy2(source / name, root / "tools" / name)
            (root / "docs/intents/001-goal.md").write_text(intent_text())
            (root / "docs/specs/001-behavior.md").write_text(spec_text())
            helper_pid_path = root / "helper.pid"
            helper = root / "tools/work-state"
            helper.write_text("#!/bin/sh\nprintf '%s' \"$$\" > \"$PID_PATH\"\nexec sleep 30\n")
            helper.chmod(0o700)
            environment = dict(os.environ)
            environment["PID_PATH"] = str(helper_pid_path)
            environment.pop("PYTHONDONTWRITEBYTECODE", None)
            process = subprocess.Popen(
                [str(root / "tools/intent-coverage"), "check"],
                cwd=root,
                env=environment,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            helper_pid = None
            helper_alive = False
            cleanup_sent = False
            try:
                helper_pid = wait_for_pid(helper_pid_path, 3)
                os.kill(process.pid, signal.SIGTERM)
                stdout, stderr = process.communicate(timeout=3)
                try:
                    os.kill(helper_pid, 0)
                except ProcessLookupError:
                    pass
                else:
                    helper_alive = True
                    os.killpg(helper_pid, signal.SIGKILL)
                    cleanup_sent = True
                self.assertFalse(helper_alive, f"owned helper {helper_pid} survived parent-only SIGTERM")
                self.assertEqual((process.returncode, stdout, stderr), (143, "", ""))
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                if helper_pid is not None and not cleanup_sent:
                    try:
                        os.kill(helper_pid, 0)
                    except ProcessLookupError:
                        pass
                    else:
                        try:
                            os.killpg(helper_pid, signal.SIGKILL)
                        except (PermissionError, ProcessLookupError):
                            pass



class GraphqlParsingTests(unittest.TestCase):
    def payload(self, pull, errors=None):
        value = {"data": {"repository": {"pullRequest": pull}}}
        if errors is not None:
            value["errors"] = errors
        return json.dumps(value).encode()

    def test_valid_and_malformed_graphql_shapes(self):
        pull = {"state": "MERGED", "mergeCommit": {"oid": COMMIT}, "mergedAt": MERGED_AT, "url": PR_URL}
        observed = coverage.parse_graphql_observation(PR_URL, 200, self.payload(pull))
        self.assertEqual(observed, merged_observation())
        malformed = [
            (403, self.payload(pull)),
            (200, b"{"),
            (200, self.payload(pull, [{"message": "denied"}])),
            (200, json.dumps({"data": {"repository": None}}).encode()),
            (200, self.payload({**pull, "url": PR_URL + "0"})),
            (200, self.payload({**pull, "mergeCommit": {"oid": COMMIT, "extra": 1}})),
            (200, self.payload({**pull, "state": []})),
            (200, self.payload({**pull, "state": {}})),
        ]
        for status, body in malformed:
            with self.subTest(status=status, body=body):
                with self.assertRaises(coverage.Refusal):
                    coverage.parse_graphql_observation(PR_URL, status, body)


class TlsHandler(http.server.BaseHTTPRequestHandler):
    response_mode = "normal"
    requests = 0
    pull_state = "MERGED"

    def do_POST(self):
        type(self).requests += 1
        length = int(self.headers["Content-Length"])
        request = json.loads(self.rfile.read(length))
        if request["variables"] != {"number": 6}:
            self.send_error(400)
            return
        body = json.dumps(
            {
                "data": {
                    "repository": {
                        "pullRequest": {
                            "state": type(self).pull_state,
                            "mergeCommit": {"oid": COMMIT},
                            "mergedAt": MERGED_AT,
                            "url": PR_URL,
                        }
                    }
                }
            },
            separators=(",", ":"),
        ).encode()
        if type(self).response_mode == "stalled":
            time.sleep(1.0)
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if type(self).response_mode == "slow":
                for byte in body:
                    self.wfile.write(bytes((byte,)))
                    self.wfile.flush()
                    time.sleep(0.015)
            else:
                self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
            pass

    def log_message(self, format_value, *arguments):
        return


@contextlib.contextmanager
def tls_server(mode="normal", state="MERGED"):
    with tempfile.TemporaryDirectory(prefix="intent-tls-fixture-") as temporary:
        root = Path(temporary)
        certificate = root / "certificate.pem"
        key = root / "key.pem"
        certificate.write_text(CERTIFICATE)
        key.write_text(PRIVATE_KEY)
        TlsHandler.response_mode = mode
        TlsHandler.pull_state = state
        TlsHandler.requests = 0
        server = http.server.ThreadingHTTPServer(("localhost", 0), TlsHandler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = ssl.TLSVersion.TLSv1_2
        context.load_cert_chain(certificate, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            yield server, certificate
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
            if thread.is_alive():
                raise RuntimeError("TLS fixture server did not stop")


@contextlib.contextmanager
def connect_stall_listener():
    with tempfile.TemporaryDirectory(prefix="intent-connect-stall-") as temporary:
        certificate = Path(temporary) / "certificate.pem"
        certificate.write_text(CERTIFICATE)
        listener = socket.socket()
        listener.bind(("localhost", 0))
        listener.listen()
        listener.settimeout(0.05)
        stop = threading.Event()
        accepted = []
        failures = []

        def stall():
            connection = None
            try:
                while not stop.is_set():
                    try:
                        connection, unused = listener.accept()
                        break
                    except socket.timeout:
                        continue
                    except OSError:
                        if stop.is_set():
                            return
                        raise
                if connection is None:
                    return
                accepted.append(connection)
                time.sleep(0.5)
            except BaseException as error:
                failures.append(error)
            finally:
                if connection is not None:
                    connection.close()

        thread = threading.Thread(target=stall)
        thread.start()
        try:
            yield listener, certificate
        finally:
            stop.set()
            listener.close()
            for connection in accepted:
                connection.close()
            thread.join(timeout=1)
            if thread.is_alive():
                raise RuntimeError("connect-stall fixture did not stop")
            if failures:
                raise failures[0]


def exit_unused_connect_stall_listener():
    with connect_stall_listener():
        pass


class RealHttpsAdapterTests(unittest.TestCase):
    def reader(self, server, certificate, limits):
        return coverage.ForgeReader(
            FixtureTokenReader(),
            limits=limits,
            transport=coverage.Transport("localhost", server.server_port, str(certificate)),
        )

    def test_real_tls_graphql_query_and_per_run_cache(self):
        with tls_server() as (server, certificate):
            limits = SCALED_LIMITS
            reader = self.reader(server, certificate, limits)
            deadline = time.monotonic() + limits.command
            self.assertEqual(reader.observe(PR_URL, deadline), merged_observation())
            self.assertEqual(reader.observe(PR_URL, deadline), merged_observation())
            self.assertEqual(TlsHandler.requests, 1)

    def test_worker_startup_does_not_consume_connect_deadline(self):
        with tls_server() as (server, certificate):
            limits = SCALED_LIMITS
            reader = coverage.ForgeReader(
                FixtureTokenReader(),
                limits=limits,
                transport=coverage.Transport("localhost", server.server_port, str(certificate)),
                process_context=DelayedStartContext(limits.connect * 1.5),
            )
            started = time.monotonic()
            self.assertEqual(
                reader.observe(PR_URL, started + limits.command),
                merged_observation(),
            )
            self.assertGreater(time.monotonic() - started, limits.connect)

    def test_malformed_environment_token_never_reaches_diagnostics(self):
        secret = "SYNTHETIC-REVIEW-SECRET\n"
        with tls_server() as (server, certificate):
            fixture = FixtureRoot()
            try:
                token_reader = coverage.TokenReader(fixture.root, environment={"GH_TOKEN": secret})
                reader = coverage.ForgeReader(
                    token_reader,
                    limits=SCALED_LIMITS,
                    transport=coverage.Transport("localhost", server.server_port, str(certificate)),
                )
                result = coverage.run(
                    "check",
                    None,
                    fixture.root,
                    FixtureNativeReader([delivered_task()]),
                    reader,
                    limits=SCALED_LIMITS,
                )
                self.assertEqual(result.exit_code, 2)
                self.assertEqual(result.stdout_lines, ())
                self.assertTrue(result.stderr_lines[0].startswith("input: GitHub authentication:"), result.stderr_lines)
                self.assertNotIn("SYNTHETIC-REVIEW-SECRET", "\n".join(result.stderr_lines))
                self.assertEqual(TlsHandler.requests, 0)
            finally:
                fixture.close()

    def test_mistyped_graphql_state_is_buffered_input_refusal(self):
        for state in ([], {}):
            with self.subTest(state=state):
                with tls_server(state=state) as (server, certificate):
                    fixture = FixtureRoot()
                    try:
                        result = coverage.run(
                            "show",
                            "001",
                            fixture.root,
                            FixtureNativeReader([delivered_task()]),
                            self.reader(server, certificate, SCALED_LIMITS),
                            limits=SCALED_LIMITS,
                        )
                        self.assertEqual(result.exit_code, 2)
                        self.assertEqual(result.stdout_lines, ())
                        self.assertEqual(result.stderr_lines, (f"input: {PR_URL}: GitHub returned an invalid pull-request state",))
                    finally:
                        fixture.close()

    def test_stalled_read_hits_idle_deadline_and_reaps_worker(self):
        with tls_server("stalled") as (server, certificate):
            limits = SCALED_LIMITS
            reader = self.reader(server, certificate, limits)
            started = time.monotonic()
            with self.assertRaisesRegex(coverage.Refusal, "read-progress deadline"):
                reader.observe(PR_URL, started + limits.command)
            self.assertLess(time.monotonic() - started, limits.command)

    def test_slow_progress_cannot_extend_observation_deadline(self):
        with tls_server("slow") as (server, certificate):
            limits = SCALED_LIMITS
            reader = self.reader(server, certificate, limits)
            started = time.monotonic()
            with self.assertRaisesRegex(coverage.Refusal, "observation deadline"):
                reader.observe(PR_URL, started + limits.command)
            elapsed = time.monotonic() - started
            self.assertGreater(elapsed, limits.idle)
            self.assertLess(elapsed, limits.command)

    def test_tls_handshake_stall_hits_connect_deadline_and_reaps_worker(self):
        with connect_stall_listener() as (listener, certificate):
            limits = SCALED_LIMITS
            reader = coverage.ForgeReader(
                FixtureTokenReader(),
                limits=limits,
                transport=coverage.Transport("localhost", listener.getsockname()[1], str(certificate)),
            )
            started = time.monotonic()
            with self.assertRaisesRegex(coverage.Refusal, "connection deadline"):
                reader.observe(PR_URL, started + limits.command)
            self.assertLess(time.monotonic() - started, limits.observation)

    def test_unexpected_accept_failure_reaches_caller_after_cleanup(self):
        original_limit = resource.getrlimit(resource.RLIMIT_NOFILE)
        descriptors = []
        released_descriptors = set()
        independent_descriptors = []
        survival_errors = []
        reused_descriptor = None
        cleanup_error = None
        client = socket.socket()
        baseline_threads = set(threading.enumerate())
        listener = None
        certificate = None

        def close_owned(owned, released=None):
            while owned:
                descriptor = owned.pop()
                os.close(descriptor)
                if released is not None:
                    released.add(descriptor)

        try:
            try:
                with self.assertRaises(OSError) as raised:
                    with connect_stall_listener() as (listener, certificate):
                        address = ("127.0.0.1", listener.getsockname()[1])
                        try:
                            resource.setrlimit(resource.RLIMIT_NOFILE, (min(64, original_limit[0]), original_limit[1]))
                            while True:
                                try:
                                    descriptors.append(os.open("/dev/null", os.O_RDONLY))
                                except OSError as error:
                                    if error.errno != errno.EMFILE:
                                        raise
                                    break
                            client.connect(address)
                            deadline = time.monotonic() + 0.5
                            while set(threading.enumerate()) != baseline_threads and time.monotonic() < deadline:
                                time.sleep(0.005)
                        finally:
                            try:
                                close_owned(descriptors, released_descriptors)
                            except OSError as error:
                                cleanup_error = error
                            finally:
                                resource.setrlimit(resource.RLIMIT_NOFILE, original_limit)
                if cleanup_error is not None:
                    raise cleanup_error
                self.assertEqual(raised.exception.errno, errno.EMFILE)
                target_descriptor = min(released_descriptors)
                while reused_descriptor is None or reused_descriptor < target_descriptor:
                    reused_descriptor = os.open("/dev/null", os.O_RDONLY)
                    independent_descriptors.append(reused_descriptor)
            finally:
                try:
                    client.close()
                finally:
                    try:
                        close_owned(descriptors)
                    finally:
                        resource.setrlimit(resource.RLIMIT_NOFILE, original_limit)
            while independent_descriptors:
                descriptor = independent_descriptors.pop()
                try:
                    os.fstat(descriptor)
                except OSError as error:
                    survival_errors.append((descriptor, error.errno))
                else:
                    os.close(descriptor)
        finally:
            try:
                close_owned(independent_descriptors)
            finally:
                resource.setrlimit(resource.RLIMIT_NOFILE, original_limit)
        self.assertEqual(reused_descriptor, min(released_descriptors))
        self.assertEqual(survival_errors, [])
        self.assertEqual(descriptors, [])
        self.assertEqual(resource.getrlimit(resource.RLIMIT_NOFILE), original_limit)
        self.assertEqual(set(threading.enumerate()), baseline_threads)
        self.assertEqual(listener.fileno(), -1)
        self.assertFalse(certificate.exists())

    def test_unused_connect_stall_listener_does_not_hold_process_open(self):
        process = multiprocessing.get_context("spawn").Process(target=exit_unused_connect_stall_listener)
        process.start()
        process.join(timeout=2)
        survived = process.is_alive()
        if survived:
            process.kill()
            process.join(timeout=1)
        self.assertFalse(process.is_alive(), "owned fixture probe survived cleanup")
        self.assertFalse(survived, "unused connect-stall fixture held its process open")
        self.assertEqual(process.exitcode, 0)


class ShellBehaviorTests(unittest.TestCase):
    def test_runner_refuses_each_missing_shell_without_writes(self):
        source = Path(__file__).resolve()
        for missing in REQUIRED_SHELLS:
            with self.subTest(missing=missing):
                with tempfile.TemporaryDirectory(prefix="intent-shell-prerequisite-") as temporary:
                    root = Path(temporary)
                    runner = root / source.name
                    shutil.copy2(source, runner)
                    shutil.copy2(source.with_name("intent_coverage.py"), root / "intent_coverage.py")
                    binaries = root / "bin"
                    binaries.mkdir()
                    available = next(shell for shell in REQUIRED_SHELLS if shell != missing)
                    shim = binaries / available
                    shim.write_text("")
                    shim.chmod(0o700)
                    before = filesystem_inventory(root)
                    environment = dict(os.environ)
                    environment["PATH"] = str(binaries)
                    environment["PYTHONDONTWRITEBYTECODE"] = "1"
                    environment.pop("PYTHONPATH", None)
                    completed = subprocess.run(
                        [sys.executable, str(runner)],
                        cwd=root,
                        env=environment,
                        text=True,
                        capture_output=True,
                        timeout=3,
                    )
                    self.assertEqual(completed.returncode, 2)
                    self.assertEqual(completed.stdout, "")
                    self.assertEqual(len(completed.stderr.splitlines()), 1)
                    self.assertIn(missing, completed.stderr)
                    self.assertEqual(filesystem_inventory(root), before)

    def test_wrong_directory_refuses_in_bash_and_zsh_without_writes(self):
        source = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory(prefix="intent-wrong-directory-") as temporary:
            root = Path(temporary)
            (root / "tools").mkdir()
            for name in ("intent-coverage", "intent_coverage.py"):
                shutil.copy2(source / name, root / "tools" / name)
            command = str(root / "tools/intent-coverage")
            before = filesystem_inventory(root)
            environment = dict(os.environ)
            environment.pop("PYTHONDONTWRITEBYTECODE", None)
            for shell in REQUIRED_SHELLS:
                with self.subTest(shell=shell):
                    completed = subprocess.run(
                        [shell, "-c", '"$1" check', shell, command],
                        cwd="/tmp",
                        env=environment,
                        text=True,
                        capture_output=True,
                        timeout=3,
                    )
                    self.assertEqual(completed.returncode, 2)
                    self.assertEqual(completed.stdout, "")
                    self.assertTrue(completed.stderr.startswith("input: checkout:"), completed.stderr)
                    self.assertNotIn("no matches found", completed.stderr)
                    self.assertEqual(filesystem_inventory(root), before)

    def test_valid_offline_root_runs_in_bash_and_zsh_without_writes(self):
        source = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory(prefix="intent-shell-root-") as temporary:
            root = Path(temporary)
            (root / "tools").mkdir()
            (root / "docs/intents").mkdir(parents=True)
            (root / "docs/specs").mkdir(parents=True)
            for name in ("intent-coverage", "intent_coverage.py"):
                shutil.copy2(source / name, root / "tools" / name)
            helper = root / "tools/work-state"
            helper.write_text("#!/usr/bin/env python3\nprint('[]')\n")
            helper.chmod(0o700)
            (root / "docs/intents/001-goal.md").write_text(intent_text())
            (root / "docs/specs/001-behavior.md").write_text(spec_text())
            before = filesystem_inventory(root)
            environment = dict(os.environ)
            environment.pop("PYTHONDONTWRITEBYTECODE", None)
            for shell in REQUIRED_SHELLS:
                with self.subTest(shell=shell):
                    checked = subprocess.run(
                        [shell, "-c", "./tools/intent-coverage check"],
                        cwd=root,
                        text=True,
                        capture_output=True,
                        timeout=3,
                        env=environment,
                    )
                    self.assertEqual((checked.returncode, checked.stdout, checked.stderr), (0, "", ""))
                    shown = subprocess.run(
                        [shell, "-c", "./tools/intent-coverage show 001"],
                        cwd=root,
                        text=True,
                        capture_output=True,
                        timeout=3,
                        env=environment,
                    )
                    self.assertEqual(shown.returncode, 0)
                    self.assertEqual(shown.stderr, "")
                    self.assertEqual(json.loads(shown.stdout), {"type": "spec", "intent_id": "001", "id": "001", "status": "accepted"})
            after = filesystem_inventory(root)
            self.assertEqual(after, before)

    def test_readme_only_intents_refuse_in_bash_and_zsh(self):
        source = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory(prefix="intent-readme-only-") as temporary:
            root = Path(temporary)
            (root / "tools").mkdir()
            (root / "docs/intents").mkdir(parents=True)
            (root / "docs/specs").mkdir(parents=True)
            for name in ("intent-coverage", "intent_coverage.py"):
                shutil.copy2(source / name, root / "tools" / name)
            (root / "docs/intents/README.md").write_text("index\n")
            (root / "docs/specs/001-behavior.md").write_text(spec_text())
            for shell in REQUIRED_SHELLS:
                with self.subTest(shell=shell):
                    completed = subprocess.run(
                        [shell, "-c", "./tools/intent-coverage check"],
                        cwd=root,
                        text=True,
                        capture_output=True,
                        timeout=3,
                    )
                    self.assertEqual(completed.returncode, 2)
                    self.assertEqual(completed.stdout, "")
                    self.assertIn("directory contains no numeric records", completed.stderr)
                    self.assertNotIn("no matches found", completed.stderr)

    def test_command_argument_errors_are_owned_input_diagnostics(self):
        root = Path(__file__).resolve().parent.parent
        command = str(root / "tools/intent-coverage")
        for arguments in ((), ("show",), ("unknown",), ("check", "extra")):
            with self.subTest(arguments=arguments):
                completed = subprocess.run(
                    [command, *arguments],
                    cwd=root,
                    text=True,
                    capture_output=True,
                    timeout=3,
                )
                self.assertEqual(completed.returncode, 2)
                self.assertEqual(completed.stdout, "")
                self.assertTrue(completed.stderr.startswith("input: command:"), completed.stderr)


def main():
    for executable in REQUIRED_SHELLS:
        if shutil.which(executable) is None:
            print(f"required executable is not on PATH: {executable}", file=sys.stderr)
            return 2
    suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if result.wasSuccessful():
        print(f"PASS: {result.testsRun} intent coverage behavioral regressions")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
