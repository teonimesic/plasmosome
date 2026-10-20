#!/usr/bin/env python3
"""List the tests a CI job's cargo test steps ignored, in the job's step summary.

Usage: ignored_tests.py LOG STEP [STEP ...]

LOG is the file every cargo test step of the job appended its output to, with `2>&1 | tee -a`.
Each STEP is NAME=OUTCOME=ARGS: the step's display name, its `steps.<id>.outcome`, and the package
arguments that step passed to `cargo test`, such as `-p plasmosome-guards`. Two crates can hold
test files of one name, so the report maps each test executable to its package by running
`cargo test --no-run --message-format=json` with those same arguments, and takes package names
from `cargo metadata`.

The summary names every ignored test with its crate and reason. It says that no test was ignored
only when every step succeeded; otherwise it names each step that did not and says the list
covers only the tests that ran. It exits 0 once the summary is written, even when a test failed,
because the test steps own that red. It exits 1 when it cannot vouch for its own list, and 2 when
its arguments are wrong. It uses only the standard library, because it runs on each runner's
system Python.
"""

import json
import os
import re
import shlex
import subprocess
import sys


USAGE = "usage: ignored_tests.py LOG NAME=OUTCOME=ARGS [NAME=OUTCOME=ARGS ...]"
OUTCOMES = {"success": "succeeded", "failure": "failed", "cancelled": "was cancelled", "skipped": "never ran"}
BUILD = ["cargo", "test", "--locked", "--no-run", "--message-format=json"]
METADATA = ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]
LIBRARY_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}

ANSI = re.compile("\x1b\\[[0-9;]*[A-Za-z]")
RUNNING = re.compile(r"^\s*Running (.+?) \((.+)\)$")
DOCTESTS = re.compile(r"^\s*Doc-tests (\S+)$")
IGNORED = re.compile(r"^test (.+?) \.\.\. ignored(?:, (.*))?$")
RESULT = re.compile(r"^test result: \S+\. \d+ passed; \d+ failed; (\d+) ignored;")


class UsageError(Exception):
    pass


class Step:
    """One cargo test step: its display name, its outcome, and the package arguments it used."""

    def __init__(self, name, outcome, arguments):
        self.name = name
        self.outcome = outcome
        self.arguments = arguments
        self.executables = {}
        self.libraries = {}


class Block:
    """The lines after one `Running` or `Doc-tests` line, up to the next one."""

    def __init__(self, kind, key, source):
        self.kind = kind
        self.key = key
        self.source = source
        self.ignored = []
        self.counted = None
        self.package = None

    def describe(self):
        if self.kind == "executable":
            return f"the Running block of {self.key} ({self.source})"
        return f"the Doc-tests block of {self.key}"


def parse_step(text):
    parts = text.split("=", 2)
    if len(parts) != 3 or not parts[0]:
        raise UsageError(f"step {text!r} is not NAME=OUTCOME=ARGS")
    name, outcome, arguments = parts
    if outcome not in OUTCOMES:
        raise UsageError(f"step {name!r} has outcome {outcome!r}, not one of {', '.join(OUTCOMES)}; "
                         "check the step id the outcome came from")
    try:
        return Step(name, outcome, shlex.split(arguments))
    except ValueError as error:
        raise UsageError(f"step {name!r} has unparseable arguments: {error}") from None


def parse_arguments(argv):
    if len(argv) < 2:
        raise UsageError("a log path and at least one step are required")
    return argv[0], [parse_step(text) for text in argv[1:]]


def parse_log(text):
    """Split a cargo test log into blocks; return them and any problems found on the way."""
    blocks, problems = [], []
    block = None
    for raw in text.splitlines():
        line = ANSI.sub("", raw)
        running = RUNNING.match(line)
        doctests = DOCTESTS.match(line)
        if running:
            block = Block("executable", running.group(2).rsplit("/", 1)[-1], running.group(1))
            blocks.append(block)
            continue
        if doctests:
            block = Block("library", doctests.group(1), doctests.group(1))
            blocks.append(block)
            continue
        ignored = IGNORED.match(line)
        result = RESULT.match(line)
        if ignored and block is None:
            problems.append(f"an ignored test comes before any Running line: {line!r}")
        elif ignored:
            block.ignored.append((ignored.group(1), ignored.group(2)))
        elif result and block is not None:
            block.counted = (block.counted or 0) + int(result.group(1))
    return blocks, problems


def package_names(metadata_text):
    """Map each workspace package id to its name, never reading the name out of the id."""
    return {package["id"]: package["name"] for package in json.loads(metadata_text)["packages"]}


def test_map(build_text, names):
    """Map test executable file names and library names to packages, from cargo's JSON lines."""
    executables, libraries, unnamed = {}, {}, []
    for line in build_text.splitlines():
        if not line.strip():
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        package = names.get(message["package_id"])
        executable = message.get("executable")
        if executable and message["profile"]["test"]:
            key = executable.rsplit("/", 1)[-1]
            if package is None:
                unnamed.append(f"test executable {key} comes from {message['package_id']}, "
                               "which cargo metadata does not name")
            else:
                executables[key] = package
        if package is not None and LIBRARY_KINDS & set(message["target"]["kind"]):
            libraries[message["target"]["name"].replace("-", "_")] = package
    return executables, libraries, unnamed


def last_line(text):
    lines = [line for line in text.splitlines() if line.strip()]
    return lines[-1] if lines else "no output"


def list_steps(steps, run):
    """Fill each step that ran with its executable and library maps; return problems and notes."""
    problems, notes = [], []
    ran = [step for step in steps if step.outcome != "skipped"]
    if not ran:
        return problems, notes
    code, output, error = run(METADATA)
    names = None
    if code == 0:
        try:
            names = package_names(output)
        except (ValueError, KeyError, TypeError) as failure:
            error = f"unreadable output: {failure}"
    if names is None:
        message = f"`cargo metadata` failed with exit {code}: {last_line(error)}"
        if any(step.outcome == "success" for step in ran):
            problems.append(message)
        else:
            notes.append(message + ".")
        return problems, notes
    for step in ran:
        code, output, error = run(BUILD + step.arguments)
        try:
            step.executables, step.libraries, unnamed = test_map(output, names)
        except (ValueError, KeyError, TypeError) as failure:
            code, error, unnamed = code or 1, f"unreadable output: {failure}", []
        problems.extend(unnamed)
        if code == 0:
            continue
        reason = f"exit {code}: {last_line(error)}"
        if step.outcome == "success":
            problems.append(f"cargo could not list the test executables of `{step.name}`, which succeeded; {reason}")
        else:
            notes.append(f"cargo could not list the test executables of `{step.name}` ({reason}).")
    return problems, notes


def check_blocks(steps, blocks):
    """Give each block its package; return what stops the report vouching for its list."""
    problems = []
    owners = {}
    for step in steps:
        for key, package in step.executables.items():
            owners.setdefault(("executable", key), []).append((step, package))
        for key, package in step.libraries.items():
            owners.setdefault(("library", key), []).append((step, package))
    for block in blocks:
        found = owners.get((block.kind, block.key), [])
        packages = sorted({package for _, package in found})
        if not packages:
            problems.append(f"{block.describe()} is in the log, but no step's cargo listing has it")
            continue
        if len(packages) > 1:
            problems.append(f"{block.describe()} belongs to more than one package: {', '.join(packages)}")
            continue
        block.package = packages[0]
        if block.counted is None:
            if all(step.outcome == "success" for step, _ in found):
                problems.append(f"{block.describe()} has no `test result:` line, though its step succeeded")
        elif block.counted != len(block.ignored):
            problems.append(f"{block.describe()} lists {len(block.ignored)} ignored tests, "
                            f"but its result lines count {block.counted}")
    return problems


def check_steps(steps, log_path, blocks):
    """A step that succeeded must have left at least one of its test executables in the log."""
    problems = []
    logged = {block.key for block in blocks or [] if block.kind == "executable"}
    for step in steps:
        if step.outcome != "success":
            continue
        if blocks is None:
            problems.append(f"`{step.name}` succeeded, but its log {log_path} does not exist")
        elif step.executables and not logged & set(step.executables):
            problems.append(f"`{step.name}` succeeded, but {log_path} has no Running line from it")
    return problems


def cell(text):
    return text.replace("|", "\\|")


def joined(parts):
    if len(parts) == 1:
        return parts[0]
    return ", ".join(parts[:-1]) + " and " + parts[-1]


def status(steps, count):
    unfinished = [f"`{step.name}` {OUTCOMES[step.outcome]}" for step in steps if step.outcome != "success"]
    if unfinished:
        return f"Not every test step succeeded: {joined(unfinished)}, so this list covers only the tests that ran."
    if count == 0:
        return "Every test step succeeded and no test was ignored."
    return f"Every test step succeeded; {count} {'test was' if count == 1 else 'tests were'} ignored."


def render(job, steps, blocks, notes):
    rows = [(block.package, name, reason) for block in blocks for name, reason in block.ignored]
    lines = [f"## Ignored tests in the {job} job", ""]
    if rows:
        lines += ["| Crate | Test | Reason |", "| --- | --- | --- |"]
        lines += [f"| {cell(package)} | {cell(name)} | {cell('no reason given' if reason is None else reason)} |"
                  for package, name, reason in rows]
        lines.append("")
    lines.append(status(steps, len(rows)))
    for note in notes:
        lines += ["", note]
    return "\n".join(lines) + "\n"


def render_refusal(job, problems):
    lines = [f"## Ignored tests in the {job} job", "", "The report cannot vouch for its list:", ""]
    lines += [f"- {problem}" for problem in problems]
    return "\n".join(lines) + "\n"


def read_log(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as log:
            return log.read()
    except FileNotFoundError:
        return None


def run_command(command):
    try:
        completed = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    except OSError as error:
        return 127, "", str(error)
    return completed.returncode, completed.stdout, completed.stderr


def main(argv, run=run_command, environ=None, stdout=None, stderr=None):
    environ = os.environ if environ is None else environ
    stdout = sys.stdout if stdout is None else stdout
    stderr = sys.stderr if stderr is None else stderr
    try:
        log_path, steps = parse_arguments(argv)
    except UsageError as error:
        print(f"ignored_tests: {error}\n{USAGE}", file=stderr)
        return 2
    text = read_log(log_path)
    blocks, problems = (None, []) if text is None else parse_log(text)
    listing_problems, notes = list_steps(steps, run)
    problems += listing_problems
    problems += check_blocks(steps, blocks or [])
    problems += check_steps(steps, log_path, blocks)
    job = environ.get("GITHUB_JOB", "local")
    summary = render_refusal(job, problems) if problems else render(job, steps, blocks or [], notes)
    stdout.write(summary)
    destination = environ.get("GITHUB_STEP_SUMMARY")
    if destination:
        with open(destination, "a", encoding="utf-8") as output:
            output.write(summary)
    for problem in problems:
        print(f"ignored_tests: {problem}", file=stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
