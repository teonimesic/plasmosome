#!/usr/bin/env python3
"""Check intent coverage judgments against repository and delivery records."""

import argparse
from dataclasses import dataclass
from datetime import datetime
import http.client
import io
import json
import multiprocessing
import os
from pathlib import Path
import re
import selectors
import signal
import ssl
import stat
import subprocess
import sys
import time


INTENT_STATES = frozenset(("draft", "approved"))
SPEC_STATES = frozenset(("draft", "accepted", "superseded"))
TASK_STATES = frozenset(("open", "planning", "in_progress", "review", "blocked", "closed"))
SERVED_VALUES = frozenset(("none", "partly", "substantially"))
DOCUMENT_NAME = re.compile(r"^[0-9]{3}.*\.md$")
THREE_DIGIT_ID = re.compile(r"^[0-9]{3}$")
NATIVE_PR = re.compile(r"^https://github\.com/teonimesic/plasmosome/pull/([1-9][0-9]*)$")
FULL_COMMIT = re.compile(r"^[0-9a-fA-F]{40}$")
RFC3339 = re.compile(
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})$"
)
GRAPHQL_QUERY = """query IntentCoveragePullRequest($number: Int!) { repository(owner: \"teonimesic\", name: \"plasmosome\") { pullRequest(number: $number) { state mergeCommit { oid } mergedAt url } } }"""
CAPTURE_LIMIT = 64 * 1024 * 1024
RESPONSE_LIMIT = 1024 * 1024
CLEANUP_RESERVE = 0.25


class Refusal(Exception):
    def __init__(self, authority, reason):
        super().__init__(reason)
        self.authority = authority
        self.reason = reason


class CommandParser(argparse.ArgumentParser):
    def error(self, message):
        raise Refusal("command", message)


@dataclass(frozen=True)
class Limits:
    connect: float = 10.0
    idle: float = 30.0
    observation: float = 60.0
    command: float = 300.0


PRODUCTION_LIMITS = Limits()


@dataclass(frozen=True)
class Document:
    kind: str
    id: str
    status: str
    intents: tuple[str, ...]
    path: Path
    relative_path: str
    lines: tuple[str, ...]
    frontmatter_end: int
    status_line: int


@dataclass(frozen=True)
class Task:
    id: str
    status: str
    spec_ids: tuple[str, ...]
    intent_ids: tuple[str, ...]
    closed_at: object
    closure: object
    external_ref: object


@dataclass(frozen=True)
class RunResult:
    exit_code: int
    stdout_lines: tuple[str, ...]
    stderr_lines: tuple[str, ...]


@dataclass(frozen=True)
class Transport:
    host: str = "api.github.com"
    port: int = 443
    cafile: str | None = None


class _ProgressRaw(io.RawIOBase):
    def __init__(self, raw, report):
        self._raw = raw
        self._report = report

    def readable(self):
        return True

    def readinto(self, buffer):
        count = self._raw.readinto(buffer)
        if count:
            self._report(count)
        return count

    def close(self):
        try:
            self._raw.close()
        finally:
            super().close()


class _ProgressSocket:
    def __init__(self, sock, report):
        self._sock = sock
        self._report = report

    def makefile(self, mode="r", buffering=None, *, encoding=None, errors=None, newline=None):
        if mode != "rb" or encoding is not None or errors is not None or newline is not None:
            return self._sock.makefile(mode, buffering, encoding=encoding, errors=errors, newline=newline)
        raw = _ProgressRaw(self._sock.makefile("rb", buffering=0), self._report)
        if buffering == 0:
            return raw
        return io.BufferedReader(raw, io.DEFAULT_BUFFER_SIZE if buffering in (None, -1) else buffering)

    def settimeout(self, value):
        self._sock.settimeout(value)

    def sendall(self, value):
        return self._sock.sendall(value)

    def shutdown(self, how):
        return self._sock.shutdown(how)

    def close(self):
        return self._sock.close()


def _send_event(channel, event):
    try:
        channel.send(event)
    except (BrokenPipeError, EOFError, OSError):
        raise SystemExit(2) from None


def _forge_worker(channel, number, token, transport, connect_timeout, read_timeout):
    connection = None
    try:
        context = ssl.create_default_context(cafile=transport.cafile)
        connection = http.client.HTTPSConnection(
            transport.host,
            transport.port,
            timeout=connect_timeout,
            context=context,
        )
        connection.connect()
        connection.sock.settimeout(read_timeout)
        _send_event(channel, ("connected",))
        connection.sock = _ProgressSocket(
            connection.sock,
            lambda count: _send_event(channel, ("progress", count)),
        )
        body = json.dumps(
            {"query": GRAPHQL_QUERY, "variables": {"number": number}},
            separators=(",", ":"),
        ).encode("utf-8")
        connection.request(
            "POST",
            "/graphql",
            body=body,
            headers={
                "Accept": "application/json",
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json",
                "User-Agent": "plasmosome-intent-coverage",
            },
        )
        response = connection.getresponse()
        received = bytearray()
        while True:
            chunk = response.read(65536)
            if not chunk:
                break
            received.extend(chunk)
            if len(received) > RESPONSE_LIMIT:
                raise ValueError(f"response exceeded {RESPONSE_LIMIT} bytes")
        _send_event(channel, ("result", response.status, bytes(received)))
    except BaseException as error:
        if not isinstance(error, SystemExit):
            _send_event(channel, ("error", f"{type(error).__name__}: {error}"))
    finally:
        if connection is not None:
            connection.close()
        channel.close()


def _remaining(deadline, clock, authority):
    value = deadline - clock()
    if value <= 0:
        raise Refusal(authority, "command deadline exceeded")
    return value


def _kill_process_group(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def _finish_owned_process(process, deadline, clock):
    while process.poll() is None and clock() < deadline:
        try:
            process.wait(timeout=min(0.02, max(0.0, deadline - clock())))
        except subprocess.TimeoutExpired:
            pass
    return process.poll() is not None


def run_owned_process(argv, cwd, deadline, clock, authority, env=None):
    try:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except OSError as error:
        raise Refusal(authority, f"could not start helper: {error}") from error
    stdout_descriptor = process.stdout.fileno()
    stderr_descriptor = process.stderr.fileno()
    output = {stdout_descriptor: bytearray(), stderr_descriptor: bytearray()}
    streams = {stdout_descriptor: process.stdout, stderr_descriptor: process.stderr}
    selector = selectors.DefaultSelector()
    for descriptor in streams:
        os.set_blocking(descriptor, False)
        selector.register(descriptor, selectors.EVENT_READ)
    active_deadline = max(clock(), deadline - CLEANUP_RESERVE)
    failure = None
    try:
        while selector.get_map() or process.poll() is None:
            now = clock()
            if now >= active_deadline:
                failure = Refusal(authority, "helper deadline exceeded")
                break
            events = selector.select(min(0.05, active_deadline - now))
            for key, unused in events:
                descriptor = key.fd
                try:
                    chunk = os.read(descriptor, 65536)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(descriptor)
                    streams[descriptor].close()
                    continue
                output[descriptor].extend(chunk)
                if len(output[descriptor]) > CAPTURE_LIMIT:
                    failure = Refusal(authority, f"helper output exceeded {CAPTURE_LIMIT} bytes")
                    break
            if failure is not None:
                break
        if failure is not None:
            _kill_process_group(process)
        if not _finish_owned_process(process, deadline, clock):
            _kill_process_group(process)
            _finish_owned_process(process, deadline, clock)
            raise Refusal(authority, "owned helper could not be reaped within the command deadline")
        if failure is not None:
            raise failure
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            pass
        else:
            _kill_process_group(process)
            raise Refusal(authority, "helper left a surviving owned process group")
        stdout = bytes(output[stdout_descriptor])
        stderr = bytes(output[stderr_descriptor])
        return process.returncode, stdout, stderr
    except BaseException:
        _kill_process_group(process)
        if not _finish_owned_process(process, deadline, clock):
            raise Refusal(authority, "owned helper cleanup did not complete within the command deadline") from None
        raise
    finally:
        selector.close()
        for stream in streams.values():
            if not stream.closed:
                stream.close()


class NativeReader:
    def __init__(self, root, clock=time.monotonic):
        self.root = root
        self.clock = clock

    def read(self, deadline):
        command = [
            str(self.root / "tools" / "work-state"),
            "list",
            "--all",
            "--limit",
            "0",
            "--json",
        ]
        code, stdout, stderr = run_owned_process(
            command,
            self.root,
            deadline,
            self.clock,
            "native store",
        )
        if code != 0:
            detail = stderr.decode("utf-8", "replace").strip() or f"work-state exited {code}"
            raise Refusal("native store", detail)
        try:
            value = json.loads(stdout.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise Refusal("native store", f"invalid JSON: {error}") from error
        if not isinstance(value, list):
            raise Refusal("native store", "work-state output must be a JSON array")
        return value


class TokenReader:
    def __init__(self, root, clock=time.monotonic, environment=None):
        self.root = root
        self.clock = clock
        self.environment = os.environ if environment is None else environment

    def read(self, deadline):
        for name in ("GH_TOKEN", "GITHUB_TOKEN"):
            token = self.environment.get(name)
            if token:
                return token
        code, stdout, stderr = run_owned_process(
            ["gh", "auth", "token", "--hostname", "github.com"],
            self.root,
            deadline,
            self.clock,
            "GitHub authentication",
            env=dict(self.environment),
        )
        if code != 0:
            detail = stderr.decode("utf-8", "replace").strip() or f"gh auth token exited {code}"
            raise Refusal("GitHub authentication", detail)
        try:
            token = stdout.decode("utf-8").strip()
        except UnicodeDecodeError as error:
            raise Refusal("GitHub authentication", "gh returned a non-UTF-8 token") from error
        if not token:
            raise Refusal("GitHub authentication", "gh returned an empty token")
        return token


class ForgeReader:
    def __init__(
        self,
        token_reader,
        clock=time.monotonic,
        limits=PRODUCTION_LIMITS,
        transport=Transport(),
        process_context=None,
    ):
        self.token_reader = token_reader
        self.clock = clock
        self.limits = limits
        self.transport = transport
        self.process_context = process_context or multiprocessing.get_context("spawn")
        self.token = None
        self.cache = {}

    def observe(self, url, command_deadline):
        if url in self.cache:
            return self.cache[url]
        match = NATIVE_PR.fullmatch(url)
        if match is None:
            raise Refusal(url, "external_ref is not a canonical repository pull-request URL")
        if self.token is None:
            self.token = self.token_reader.read(command_deadline)
        number = int(match.group(1))
        observation_deadline = min(command_deadline, self.clock() + self.limits.observation)
        connect_deadline = min(observation_deadline, self.clock() + self.limits.connect)
        receive, send = self.process_context.Pipe(duplex=False)
        process = self.process_context.Process(
            target=_forge_worker,
            args=(
                send,
                number,
                self.token,
                self.transport,
                min(self.limits.connect, max(0.001, observation_deadline - self.clock())),
                max(0.001, observation_deadline - self.clock()),
            ),
        )
        process.start()
        send.close()
        connected = False
        last_progress = None
        result = None
        failure = None
        cleanup_reserve = min(CLEANUP_RESERVE, self.limits.observation / 10)
        active_deadline = max(self.clock(), observation_deadline - cleanup_reserve)
        try:
            while result is None and failure is None:
                now = self.clock()
                bounds = [active_deadline]
                if not connected:
                    bounds.append(connect_deadline)
                elif last_progress is not None:
                    bounds.append(last_progress + self.limits.idle)
                event_deadline = min(bounds)
                if now >= event_deadline:
                    if not connected and event_deadline == connect_deadline:
                        failure = Refusal(url, "connection deadline exceeded")
                    elif connected and last_progress is not None and event_deadline == last_progress + self.limits.idle:
                        failure = Refusal(url, "read-progress deadline exceeded")
                    else:
                        failure = Refusal(url, "observation deadline exceeded")
                    break
                if receive.poll(min(0.05, event_deadline - now)):
                    try:
                        event = receive.recv()
                    except EOFError:
                        failure = Refusal(url, "GitHub worker exited without a result")
                        break
                    kind = event[0]
                    if kind == "connected":
                        connected = True
                        last_progress = self.clock()
                    elif kind == "progress":
                        if connected and event[1] > 0:
                            last_progress = self.clock()
                    elif kind == "result":
                        result = event[1:]
                    elif kind == "error":
                        failure = Refusal(url, event[1])
                    else:
                        failure = Refusal(url, "GitHub worker returned an unknown event")
                elif not process.is_alive() and not receive.poll():
                    failure = Refusal(url, "GitHub worker exited without a result")
            if failure is not None:
                if process.is_alive():
                    process.kill()
            process.join(timeout=max(0.0, observation_deadline - self.clock()))
            if process.is_alive():
                process.kill()
                process.join(timeout=max(0.0, observation_deadline - self.clock()))
            if process.is_alive() or process.exitcode is None:
                raise Refusal(url, "owned GitHub worker could not be reaped within its deadline")
            if failure is not None:
                raise failure
            if process.exitcode != 0:
                raise Refusal(url, f"GitHub worker exited {process.exitcode}")
            status, body = result
            observation = parse_graphql_observation(url, status, body)
            self.cache[url] = observation
            return observation
        except BaseException:
            if process.is_alive():
                process.kill()
                process.join(timeout=max(0.0, observation_deadline - self.clock()))
            if process.is_alive():
                raise Refusal(url, "owned GitHub worker cleanup did not complete within its deadline") from None
            raise
        finally:
            receive.close()


def valid_rfc3339(value):
    if not isinstance(value, str) or RFC3339.fullmatch(value) is None:
        return False
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return False
    return True


def parse_graphql_observation(url, status, body):
    if not isinstance(status, int) or status < 200 or status >= 300:
        raise Refusal(url, f"GitHub returned HTTP {status}")
    try:
        payload = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise Refusal(url, f"GitHub returned invalid JSON: {error}") from error
    if not isinstance(payload, dict):
        raise Refusal(url, "GitHub response must be an object")
    if payload.get("errors"):
        raise Refusal(url, "GitHub GraphQL returned errors")
    data = payload.get("data")
    if not isinstance(data, dict):
        raise Refusal(url, "GitHub response has no data object")
    repository = data.get("repository")
    if not isinstance(repository, dict):
        raise Refusal(url, "GitHub response has no repository object")
    pull = repository.get("pullRequest")
    if not isinstance(pull, dict):
        raise Refusal(url, "GitHub response has no pull request object")
    if set(pull) != {"state", "mergeCommit", "mergedAt", "url"}:
        raise Refusal(url, "GitHub pull-request fields are incomplete or unexpected")
    state = pull["state"]
    if state not in {"OPEN", "CLOSED", "MERGED"}:
        raise Refusal(url, "GitHub returned an invalid pull-request state")
    if pull["url"] != url:
        raise Refusal(url, "GitHub returned a different pull-request URL")
    merge_commit = pull["mergeCommit"]
    if merge_commit is not None:
        if not isinstance(merge_commit, dict) or set(merge_commit) != {"oid"} or not isinstance(merge_commit["oid"], str):
            raise Refusal(url, "GitHub returned a malformed merge commit")
        merge_commit = merge_commit["oid"]
    merged_at = pull["mergedAt"]
    if merged_at is not None and not isinstance(merged_at, str):
        raise Refusal(url, "GitHub returned a malformed merge time")
    return {"state": state, "merge_commit": merge_commit, "merged_at": merged_at, "url": pull["url"]}


def _scalar(value):
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
        quote = value[0]
        inner = value[1:-1]
        if quote == "'":
            return inner.replace("''", "'")
        try:
            decoded = json.loads(value)
        except json.JSONDecodeError:
            return None
        return decoded if isinstance(decoded, str) else None
    return value


def _flow(value):
    value = value.strip()
    if not value.startswith("[") or not value.endswith("]"):
        return None
    body = value[1:-1].strip()
    if not body:
        return ()
    result = []
    for item in body.split(","):
        parsed = _scalar(item)
        if parsed is None or THREE_DIGIT_ID.fullmatch(parsed) is None:
            return None
        result.append(parsed)
    return tuple(result)


def parse_document(path, text, kind, root):
    relative = path.relative_to(root).as_posix()
    lines = tuple(text.splitlines())
    if not lines or lines[0] != "---":
        raise Refusal(relative, "numeric document must start with a frontmatter delimiter")
    try:
        end = lines.index("---", 1)
    except ValueError as error:
        raise Refusal(relative, "numeric document has no closing frontmatter delimiter") from error
    occurrences = {"id": [], "status": [], "intents": []}
    field_pattern = re.compile(r"^([A-Za-z_][A-Za-z0-9_-]*):[ \t]*(.*)$")
    for index, line in enumerate(lines[1:end], 1):
        match = field_pattern.fullmatch(line)
        if match and match.group(1) in occurrences:
            occurrences[match.group(1)].append((index, match.group(2)))
    for name in ("id", "status"):
        if len(occurrences[name]) != 1:
            raise Refusal(relative, f"frontmatter must contain exactly one {name} field")
    identity = _scalar(occurrences["id"][0][1])
    if identity is None or THREE_DIGIT_ID.fullmatch(identity) is None:
        raise Refusal(relative, "id must be a three-digit string")
    state = _scalar(occurrences["status"][0][1])
    legal = INTENT_STATES if kind == "intent" else SPEC_STATES
    if state not in legal:
        raise Refusal(relative, f"status must be one of {','.join(sorted(legal))}")
    if kind == "spec":
        if len(occurrences["intents"]) != 1:
            raise Refusal(relative, "spec frontmatter must contain exactly one intents field")
        intents = _flow(occurrences["intents"][0][1])
        if intents is None:
            raise Refusal(relative, "intents must be a one-line list of three-digit strings")
    else:
        if occurrences["intents"]:
            raise Refusal(relative, "intent frontmatter must not declare spec intent links")
        intents = ()
    return Document(
        kind,
        identity,
        state,
        intents,
        path,
        relative,
        lines,
        end,
        occurrences["status"][0][0],
    )


def _read_kind(root, kind, deadline, clock):
    directory = root / "docs" / ("intents" if kind == "intent" else "specs")
    try:
        metadata = directory.lstat()
    except OSError as error:
        raise Refusal(directory.relative_to(root).as_posix(), f"missing or unreadable directory: {error}") from error
    if not stat.S_ISDIR(metadata.st_mode):
        raise Refusal(directory.relative_to(root).as_posix(), "authority path is not a directory")
    documents = {}
    candidates = sorted(
        (entry for entry in directory.iterdir() if DOCUMENT_NAME.fullmatch(entry.name)),
        key=lambda entry: entry.name,
    )
    if not candidates:
        raise Refusal(directory.relative_to(root).as_posix(), "directory contains no numeric records")
    for path in candidates:
        _remaining(deadline, clock, path.relative_to(root).as_posix())
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode):
            raise Refusal(path.relative_to(root).as_posix(), "numeric document is not a regular file")
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            raise Refusal(path.relative_to(root).as_posix(), f"could not read UTF-8 document: {error}") from error
        document = parse_document(path, text, kind, root)
        if document.id in documents:
            raise Refusal(document.id, f"duplicate {kind} id in {documents[document.id].relative_path} and {document.relative_path}")
        documents[document.id] = document
    return documents


def read_documents(root, deadline, clock):
    intents = _read_kind(root, "intent", deadline, clock)
    specs = _read_kind(root, "spec", deadline, clock)
    for spec in specs.values():
        for intent_id in spec.intents:
            if intent_id not in intents:
                raise Refusal(spec.relative_path, f"intents names missing intent {intent_id}")
        if spec.status == "accepted":
            if not spec.intents:
                raise Refusal(spec.relative_path, "accepted spec must name at least one intent")
            unapproved = [identity for identity in spec.intents if intents[identity].status != "approved"]
            if unapproved:
                raise Refusal(spec.relative_path, f"accepted spec reaches unapproved intent {unapproved[0]}")
    return intents, specs


def _link_array(task_id, metadata, name):
    if name not in metadata or not isinstance(metadata[name], list):
        raise Refusal(task_id, f"metadata.{name} must be an array")
    result = []
    for value in metadata[name]:
        if not isinstance(value, str) or THREE_DIGIT_ID.fullmatch(value) is None:
            raise Refusal(task_id, f"metadata.{name} must contain only three-digit strings")
        result.append(value)
    return tuple(result)


def parse_tasks(rows, intents, specs):
    if not isinstance(rows, list):
        raise Refusal("native store", "native input must be an array")
    tasks = {}
    for row in rows:
        if not isinstance(row, dict):
            raise Refusal("native store", "every native row must be an object")
        identity = row.get("id")
        if not isinstance(identity, str) or identity == "":
            raise Refusal("native store", "every native row must have a nonempty literal id")
        if identity in tasks:
            raise Refusal(identity, "duplicate native id")
        status_value = row.get("status")
        if not isinstance(status_value, str) or status_value not in TASK_STATES:
            raise Refusal(identity, "status must be one of open,planning,in_progress,review,blocked,closed")
        metadata = row.get("metadata")
        if not isinstance(metadata, dict):
            raise Refusal(identity, "metadata must be an object")
        spec_ids = _link_array(identity, metadata, "spec_ids")
        intent_ids = _link_array(identity, metadata, "intent_ids")
        for spec_id in spec_ids:
            if spec_id not in specs:
                raise Refusal(identity, f"metadata.spec_ids names missing spec {spec_id}")
        for intent_id in intent_ids:
            if intent_id not in intents:
                raise Refusal(identity, f"metadata.intent_ids names missing intent {intent_id}")
        if spec_ids:
            copied = []
            for spec_id in spec_ids:
                for intent_id in specs[spec_id].intents:
                    if intent_id not in copied:
                        copied.append(intent_id)
            if tuple(copied) != intent_ids:
                raise Refusal(identity, "metadata.intent_ids does not equal the ordered union of linked spec intents")
        external_ref = row.get("external_ref")
        if external_ref is not None and not isinstance(external_ref, str):
            raise Refusal(identity, "external_ref must be a string when present")
        tasks[identity] = Task(
            identity,
            status_value,
            spec_ids,
            intent_ids,
            row.get("closed_at"),
            metadata.get("closure"),
            external_ref,
        )
    return tasks


def task_reaches(task, intent_id, specs):
    return intent_id in task.intent_ids or any(intent_id in specs[spec_id].intents for spec_id in task.spec_ids)


def parse_closure(task):
    if task.status != "closed":
        return {"disposition": "open"}
    if not valid_rfc3339(task.closed_at):
        raise Refusal(task.id, "closed task has an invalid closed_at timestamp")
    closure = task.closure
    if not isinstance(closure, dict):
        raise Refusal(task.id, "closed task has no valid metadata.closure object")
    kind = closure.get("kind")
    expected = {"kind", "closed_at"} if kind == "delivered" else {"kind", "closed_at", "reason"}
    if kind not in {"delivered", "cancelled"} or set(closure) != expected:
        raise Refusal(task.id, "metadata.closure has unsupported, missing or extra keys")
    if not isinstance(closure["closed_at"], str) or closure["closed_at"] != task.closed_at:
        raise Refusal(task.id, "metadata.closure.closed_at does not exactly match native closed_at")
    if kind == "cancelled":
        reason = closure["reason"]
        if not isinstance(reason, str) or not reason.strip() or reason.strip() == "Closed":
            raise Refusal(task.id, "cancelled closure requires an explicit non-generic reason")
        return {"disposition": "cancelled", "reason": reason}
    return {"disposition": "delivered"}


def validate_evidence(tasks, intents, specs, forge_reader, deadline, clock):
    dispositions = {}
    pending = {}
    for task in tasks.values():
        if not any(task_reaches(task, intent_id, specs) for intent_id in intents):
            continue
        disposition = parse_closure(task)
        dispositions[task.id] = disposition
        if disposition["disposition"] == "open":
            continue
        reference = task.external_ref
        if disposition["disposition"] == "delivered":
            if not isinstance(reference, str) or not reference:
                raise Refusal(task.id, "delivered closure requires a canonical external_ref")
            if NATIVE_PR.fullmatch(reference) is None:
                raise Refusal(task.id, "delivered external_ref is not a canonical repository pull-request URL")
            pending[reference] = None
        elif reference:
            if NATIVE_PR.fullmatch(reference) is None:
                raise Refusal(task.id, "cancelled external_ref is not a canonical repository pull-request URL")
            pending[reference] = None
    for reference in sorted(pending):
        _remaining(deadline, clock, reference)
        pending[reference] = forge_reader.observe(reference, deadline)
    for task in tasks.values():
        disposition = dispositions.get(task.id)
        if disposition is None or disposition["disposition"] == "open":
            continue
        reference = task.external_ref
        if disposition["disposition"] == "delivered":
            observation = pending[reference]
            if (
                observation["state"] != "MERGED"
                or not isinstance(observation["merge_commit"], str)
                or FULL_COMMIT.fullmatch(observation["merge_commit"]) is None
                or not valid_rfc3339(observation["merged_at"])
            ):
                raise Refusal(reference, "delivered closure lacks a matching merged PR, full commit, and merge time")
            disposition.update(
                pr_url=reference,
                merge_commit=observation["merge_commit"],
                merged_at=observation["merged_at"],
            )
        elif reference:
            observation = pending[reference]
            if observation["state"] != "CLOSED" or observation["merge_commit"] is not None or observation["merged_at"] is not None:
                raise Refusal(reference, "cancelled closure PR must be closed and unmerged")
            disposition["pr_url"] = reference
    return dispositions


def derive(intent_id, intents, specs, tasks, dispositions):
    rows = []
    matching_specs = sorted(
        (spec for spec in specs.values() if intent_id in spec.intents),
        key=lambda spec: spec.id,
    )
    emitted = set()
    for spec in matching_specs:
        rows.append({"type": "spec", "intent_id": intent_id, "id": spec.id, "status": spec.status})
        matching_tasks = sorted(
            (task for task in tasks.values() if spec.id in task.spec_ids and task.id not in emitted),
            key=lambda task: task.id,
        )
        for task in matching_tasks:
            row = {
                "type": "task",
                "intent_id": intent_id,
                "spec_id": spec.id,
                "id": task.id,
                "status": task.status,
                **dispositions[task.id],
            }
            rows.append(row)
            emitted.add(task.id)
    direct = sorted(
        (task for task in tasks.values() if intent_id in task.intent_ids and task.id not in emitted),
        key=lambda task: task.id,
    )
    for task in direct:
        rows.append(
            {
                "type": "task",
                "intent_id": intent_id,
                "spec_id": None,
                "id": task.id,
                "status": task.status,
                **dispositions[task.id],
            }
        )
        emitted.add(task.id)
    return rows


def coverage_fault(intent, rows):
    served = [(index, line) for index, line in enumerate(intent.lines) if line.startswith("served:")]
    if len(served) != 1:
        return 3
    index, line = served[0]
    match = re.fullmatch(r"served:[ \t]*(none|partly|substantially)[ \t]*", line)
    if index >= intent.frontmatter_end or index != intent.status_line + 1 or match is None:
        return 3
    value = match.group(1)
    landed = any(row.get("type") == "task" and row.get("disposition") == "delivered" for row in rows)
    if value == "none" and landed:
        return 1
    if value == "substantially" and not landed:
        return 2
    return None


def render_show(rows):
    return tuple(json.dumps(row, separators=(",", ":"), ensure_ascii=False) for row in rows)


def run(command, requested_id, root, native_reader, forge_reader, clock=time.monotonic, limits=PRODUCTION_LIMITS):
    started = clock()
    deadline = started + limits.command
    try:
        intents, specs = read_documents(root, deadline, clock)
        if command == "show" and (requested_id is None or THREE_DIGIT_ID.fullmatch(requested_id) is None or requested_id not in intents):
            raise Refusal(requested_id or "show", "requested intent must be an existing three-digit intent ID")
        native_rows = native_reader.read(deadline)
        _remaining(deadline, clock, "native store")
        tasks = parse_tasks(native_rows, intents, specs)
        dispositions = validate_evidence(tasks, intents, specs, forge_reader, deadline, clock)
        _remaining(deadline, clock, "coverage")
        if command == "show":
            rows = derive(requested_id, intents, specs, tasks, dispositions)
            _remaining(deadline, clock, "coverage")
            return RunResult(0, render_show(rows), ())
        faults = []
        for intent_id in sorted(intents):
            _remaining(deadline, clock, intents[intent_id].relative_path)
            intent = intents[intent_id]
            fault = coverage_fault(intent, derive(intent_id, intents, specs, tasks, dispositions))
            if fault is not None:
                faults.append(f"{intent.relative_path}: fault{fault}")
        _remaining(deadline, clock, "coverage")
        return RunResult(1 if faults else 0, tuple(faults), ())
    except Refusal as error:
        return RunResult(2, (), (f"input: {error.authority}: {error.reason}",))


def _arguments(argv):
    parser = CommandParser(prog="intent-coverage", description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("check", help="check every intent coverage judgment")
    show = subparsers.add_parser("show", help="show derived work for one intent")
    show.add_argument("intent_id", metavar="INTENT_ID")
    options = parser.parse_args(argv)
    return options.command, getattr(options, "intent_id", None)


def main(argv=None):
    try:
        command, requested_id = _arguments(sys.argv[1:] if argv is None else argv)
        root = Path(__file__).resolve().parent.parent
        if Path.cwd().resolve() != root:
            raise Refusal("checkout", f"run from repository root {root}")
        limits = PRODUCTION_LIMITS
        clock = time.monotonic
        native_reader = NativeReader(root, clock)
        token_reader = TokenReader(root, clock)
        forge_reader = ForgeReader(token_reader, clock, limits)
        result = run(command, requested_id, root, native_reader, forge_reader, clock, limits)
    except Refusal as error:
        result = RunResult(2, (), (f"input: {error.authority}: {error.reason}",))
    except (OSError, UnicodeError, ValueError) as error:
        result = RunResult(2, (), (f"input: command: {error}",))
    for line in result.stdout_lines:
        print(line)
    for line in result.stderr_lines:
        print(line, file=sys.stderr)
    return result.exit_code


if __name__ == "__main__":
    sys.exit(main())
