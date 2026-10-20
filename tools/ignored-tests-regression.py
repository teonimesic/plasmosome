#!/usr/bin/env python3
"""Regressions for tools/ignored_tests.py, the ignored-test report of CI's gates and macos jobs.

Each case names the bug it catches. Cargo never runs: a fake answers the report's two commands
with fixed JSON, and every log is written here line by line in libtest's and cargo's format.
"""

import io
import json
import sys
import tempfile
import traceback
from pathlib import Path

import ignored_tests


TESTS = "--workspace --exclude plasmosome-guards"
GUARDS = "-p plasmosome-guards"
BUILD = ["cargo", "test", "--locked", "--no-run", "--message-format=json"]
METADATA_COMMAND = ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]

CORE = "path+file:///w/crates/core#0.1.0"
MEMBRANE = "path+file:///w/crates/membrane#plasmosome-membrane@0.1.0"
GUARDS_ID = "path+file:///w/crates/plasmosome-guards#0.1.0"
SERDE = "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.228"

METADATA = json.dumps({
    "packages": [
        {"id": CORE, "name": "plasmosome-core"},
        {"id": MEMBRANE, "name": "plasmosome-membrane"},
        {"id": GUARDS_ID, "name": "plasmosome-guards"},
    ],
    "workspace_members": [CORE, MEMBRANE, GUARDS_ID],
    "version": 1,
})

NONE = "Every test step succeeded and no test was ignored."
COVERS = "covers only the tests that ran"
HEADER = "| Crate | Test | Reason |"
REFUSED = "The report cannot vouch for its list"
ARGS_HINT = "check that the `ignored tests` step passes the same package arguments as that test step"
TEE_HINT = "check the step's `tee -a` path"
TESTS_FAILED_HINT = "`tests` failed; a failing test that prints cargo output can cause this, so read that step first"
ESC = chr(27)
BACKSLASH = chr(92)


def artifact(package, name, kind, executable=None, test=True):
    return json.dumps({
        "reason": "compiler-artifact",
        "package_id": package,
        "target": {"kind": [kind], "crate_types": [kind], "name": name},
        "profile": {"test": test},
        "executable": executable,
        "fresh": True,
    })


FINISHED = json.dumps({"reason": "build-finished", "success": True})

TESTS_BUILD = [
    artifact(SERDE, "serde", "lib", test=False),
    artifact(CORE, "plasmosome_core", "lib", test=False),
    artifact(CORE, "plasmosome_core", "lib", "/w/target/debug/deps/plasmosome_core-c0e1"),
    artifact(CORE, "daemon_cli", "test", "/w/target/debug/deps/daemon_cli-c0e2"),
    artifact(CORE, "plasmosomed", "bin", "/w/target/debug/plasmosomed", test=False),
    artifact(MEMBRANE, "plasmosome_membrane", "lib", test=False),
    artifact(MEMBRANE, "plasmosome_membrane", "lib", "/w/target/debug/deps/plasmosome_membrane-3e31"),
    artifact(MEMBRANE, "daemon_cli", "test", "/w/target/debug/deps/daemon_cli-3e32"),
    FINISHED,
]

GUARDS_BUILD = [
    artifact(CORE, "plasmosome_core", "lib", test=False),
    artifact(GUARDS_ID, "plasmosome_guards", "lib", test=False),
    artifact(GUARDS_ID, "plasmosome_guards", "lib", "/w/target/debug/deps/plasmosome_guards-6a71"),
    artifact(GUARDS_ID, "workspace_guards", "test", "/w/target/debug/deps/workspace_guards-6a72"),
    artifact(GUARDS_ID, "workspace_roots", "test", "/w/target/debug/deps/workspace_roots-6a73"),
    FINISHED,
]


def running(source, executable):
    return [f"     Running {source} ({executable})", ""]


def doctests(library):
    return [f"   Doc-tests {library}", ""]


def result_line(passed, failed, ignored):
    verdict = "FAILED" if failed else "ok"
    return (f"test result: {verdict}. {passed} passed; {failed} failed; {ignored} ignored; "
            "0 measured; 0 filtered out; finished in 0.01s")


def tests(*lines, ignored=None):
    """A libtest run of these test lines, ending with the result line that counts them."""
    counted = sum(" ... ignored" in line for line in lines) if ignored is None else ignored
    failed = sum(line.endswith(" ... FAILED") for line in lines)
    passed = len(lines) - counted - failed
    return [f"running {len(lines)} tests", *lines, "", result_line(passed, failed, counted), ""]


CORE_UNIT = (running("unittests src/lib.rs", "target/debug/deps/plasmosome_core-c0e1")
             + tests("test session_log::tests::appends ... ok"))
CORE_CLI = (running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
            + tests("test answers_usage ... ok"))
MEMBRANE_UNIT = (running("unittests src/lib.rs", "target/debug/deps/plasmosome_membrane-3e31")
                 + tests("test vmm::tests::reaps_the_child ... ok"))
MEMBRANE_CLI = (running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-3e32")
                + tests("test answers_usage ... ok"))
CORE_DOCS = doctests("plasmosome_core") + tests()
GUARDS_UNIT = (running("unittests src/lib.rs", "target/debug/deps/plasmosome_guards-6a71")
               + tests("test workspace::tests::finds_the_root ... ok"))
GUARDS_RUN = (running("tests/workspace_guards.rs", "target/debug/deps/workspace_guards-6a72")
              + tests("test ci_matrix_matches_workspace_members ... ok"))
GUARDS_ROOTS = (running("tests/workspace_roots.rs", "target/debug/deps/workspace_roots-6a73")
                + tests("test prebuilt_consumers_cannot_certify_a_copy ... ok"))
GUARDS_DOCS = doctests("plasmosome_guards") + tests()


def tests_log(core_unit=CORE_UNIT, core_cli=CORE_CLI, membrane_unit=MEMBRANE_UNIT, membrane_cli=MEMBRANE_CLI,
              core_docs=CORE_DOCS):
    """The tests step's whole output, every block clean unless named."""
    return core_unit + core_cli + membrane_unit + membrane_cli + core_docs


def guards_log(roots=GUARDS_ROOTS):
    """The workspace guards step's whole output, the workspace_roots block clean unless given."""
    return GUARDS_UNIT + GUARDS_RUN + roots + GUARDS_DOCS


def log_of(roots=GUARDS_ROOTS, **blocks):
    """Both steps' output in one log, as the two tee -a pipelines leave it."""
    return tests_log(**blocks) + guards_log(roots)


CLEAN = log_of()

FAILING_CORE_UNIT = (running("unittests src/lib.rs", "target/debug/deps/plasmosome_core-c0e1") + [
    "running 2 tests",
    "test session_log::tests::appends ... FAILED",
    "test session_log::tests::reads ... ok",
    "",
    "failures:",
    "",
    "---- session_log::tests::appends stdout ----",
    "",
    "thread 'session_log::tests::appends' panicked at crates/core/src/session_log.rs:9:5:",
    "assertion failed: appended",
    "",
    "failures:",
    "    session_log::tests::appends",
    "",
    result_line(1, 1, 0),
    "",
    "error: test failed, to rerun pass `-p plasmosome-core --lib`",
])

FAILING_CORE_CLI_MISCOUNTED = (running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2") + [
    "running 3 tests",
    "test boots ... ignored, needs a hypervisor",
    "test answers_usage ... FAILED",
    "",
    "failures:",
    "",
    "---- answers_usage stdout ----",
    "",
    "thread 'answers_usage' panicked at crates/core/tests/daemon_cli.rs:12:5:",
    "usage missing",
    "",
    "failures:",
    "    answers_usage",
    "",
    result_line(0, 1, 2),
    "",
    "error: test failed, to rerun pass `-p plasmosome-core --test daemon_cli`",
])


def nested_failure(executable):
    """workspace_roots failing with a nested cargo run in its panic message, cut from a real red run.

    The nested run's stdout comes first and ends with its own result line. Its stderr follows,
    with a Running line naming EXECUTABLE. The outer block's own result line comes after that.
    """
    return running("tests/workspace_roots.rs", "target/debug/deps/workspace_roots-6a73") + [
        "running 2 tests",
        "test held_names_stay_held ... ignored, needs a registry",
        "test prebuilt_consumers_cannot_certify_a_copy ... FAILED",
        "",
        "failures:",
        "",
        "---- prebuilt_consumers_cannot_certify_a_copy stdout ----",
        "workspace roots: copy-only violation: only_the_held_names_are_publishable_to_a_registry: observed",
        "",
        "thread 'prebuilt_consumers_cannot_certify_a_copy' (25089997) panicked at "
        "crates/plasmosome-guards/tests/workspace_roots.rs:506:9:",
        "outside Cargo manifest selection after B rebuild: only_the_held_names_are_publishable_to_a_registry",
        "",
        "running 1 test",
        "test only_the_held_names_are_publishable_to_a_registry ... ok",
        "",
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 11 filtered out; finished in 0.10s",
        "",
        "",
        "    Blocking waiting for file lock on package cache",
        "    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.26s",
        f"     Running tests/workspace_guards.rs ({executable})",
        "",
        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
        "",
        "",
        "failures:",
        "    prebuilt_consumers_cannot_certify_a_copy",
        "",
        result_line(0, 1, 1),
        "",
        "error: test failed, to rerun pass `-p plasmosome-guards --test workspace_roots`",
    ]


NESTED = "/private/var/folders/9h/T/.tmpL9n6pT/b/target/rebuilt/debug/deps"


class Outcome:
    def __init__(self, code, summary, stdout, stderr, calls):
        self.code = code
        self.summary = summary
        self.stdout = stdout
        self.stderr = stderr
        self.calls = calls

    def __repr__(self):
        return (f"exit {self.code}\n--- summary\n{self.summary}--- stdout\n{self.stdout}"
                f"--- stderr\n{self.stderr}--- calls\n{self.calls}")


EARLIER = "Earlier step output.\n"


def report(log, outcomes=("success", "success"), builds=None, metadata=(0, METADATA), job="gates",
           argv=None):
    """Run the report's main over LOG, with a fake cargo answering from BUILDS and METADATA."""
    if builds is None:
        builds = {TESTS: (0, TESTS_BUILD), GUARDS: (0, GUARDS_BUILD)}
    with tempfile.TemporaryDirectory() as scratch:
        path = Path(scratch) / "cargo-test.log"
        if log is not None:
            path.write_text("".join(line + "\n" for line in log))
        summary = Path(scratch) / "summary.md"
        summary.write_text(EARLIER)
        calls = []

        def run(command):
            calls.append(list(command))
            if list(command) == METADATA_COMMAND:
                code, text = metadata
                return code, text + "\n", "" if code == 0 else "error: could not read the manifest"
            if list(command[:len(BUILD)]) == BUILD:
                code, rows = builds.get(" ".join(command[len(BUILD):]), (101, []))
                return code, "".join(row + "\n" for row in rows), "" if code == 0 else "error: could not compile"
            return 127, "", f"unexpected command {command}"

        if argv is None:
            arguments = [str(path), f"tests={outcomes[0]}={TESTS}", f"workspace guards={outcomes[1]}={GUARDS}"]
        else:
            arguments = argv(str(path))
        stdout, stderr = io.StringIO(), io.StringIO()
        code = ignored_tests.main(arguments, run=run,
                                  environ={"GITHUB_STEP_SUMMARY": str(summary), "GITHUB_JOB": job},
                                  stdout=stdout, stderr=stderr)
        return Outcome(code, summary.read_text(), stdout.getvalue(), stderr.getvalue(), calls)


def one_file_name_in_two_crates_lists_each_under_its_own_crate():
    """Case 1. Catches keying by source path: there are two tests/daemon_cli.rs files."""
    log = log_of(core_cli=(running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                           + tests("test boots ... ignored, needs a hypervisor", "test answers_usage ... ok")),
                 membrane_cli=(running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-3e32")
                               + tests("test boots ... ignored, needs a hypervisor")))
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | boots | needs a hypervisor |" in result.summary, result
    assert "| plasmosome-membrane | boots | needs a hypervisor |" in result.summary, result


def a_doctest_is_listed_under_its_own_crate_with_no_reason_given():
    """Case 2. Catches a doctest given the previous binary's crate, and dropping a reasonless line."""
    log = log_of(core_docs=doctests("plasmosome_core")
                 + tests("test crates/core/src/lib.rs - parse (line 7) ... ignored"))
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | crates/core/src/lib.rs - parse (line 7) | no reason given |" in result.summary, result
    assert "plasmosome-membrane |" not in result.summary, result


def a_reason_is_kept_verbatim():
    """Case 3. Catches splitting the reason on its commas or dots."""
    reason = "needs a booted cell, a signed kernel... and, later, a network"
    log = log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                 + tests(f"test boots ... ignored, {reason}"))
    result = report(log)
    assert result.code == 0, result
    assert f"| plasmosome-core | boots | {reason} |" in result.summary, result


def package_names_come_from_metadata_not_from_the_id():
    """Case 4. CORE's id names a directory, MEMBRANE's carries #name@version. Catches splitting the id."""
    log = log_of(core_cli=(running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                           + tests("test boots ... ignored, needs a hypervisor")),
                 membrane_cli=(running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-3e32")
                               + tests("test boots ... ignored, needs a hypervisor")))
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | boots |" in result.summary, result
    assert "| plasmosome-membrane | boots |" in result.summary, result
    assert "| core |" not in result.summary, result

    stranger = "path+file:///w/crates/stranger#0.1.0"
    build = TESTS_BUILD + [artifact(stranger, "stranger", "test", "/w/target/debug/deps/stranger-5a51")]
    log = log_of(core_unit=CORE_UNIT + running("tests/stranger.rs", "target/debug/deps/stranger-5a51")
                 + tests("test x ... ok"))
    unnamed = report(log, builds={TESTS: (0, build), GUARDS: (0, GUARDS_BUILD)})
    assert unnamed.code == 1, unnamed
    assert "stranger-5a51 (tests/stranger.rs) is in the log, but no step's cargo listing has it" in unnamed.stderr, unnamed


def relative_and_absolute_running_paths_both_map():
    """Case 5. Catches matching the whole path, which cargo prints either way."""
    log = log_of(core_cli=(running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                           + tests("test boots ... ignored, needs a hypervisor")),
                 membrane_cli=(running("tests/daemon_cli.rs", "/w/target/debug/deps/daemon_cli-3e32")
                               + tests("test boots ... ignored, needs a hypervisor")))
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | boots |" in result.summary, result
    assert "| plasmosome-membrane | boots |" in result.summary, result


def the_status_sentence_says_none_or_counts():
    """Case 6. Catches a wrong or missing status sentence when every step succeeded."""
    clean = report(CLEAN)
    assert clean.code == 0, clean
    assert NONE in clean.summary, clean
    assert HEADER not in clean.summary, clean

    two = report(log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                        + tests("test boots ... ignored, needs a hypervisor",
                                "test halts ... ignored, needs a hypervisor")))
    assert two.code == 0, two
    assert "Every test step succeeded; 2 tests were ignored." in two.summary, two
    assert NONE not in two.summary, two
    assert HEADER in two.summary, two

    one = report(log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                        + tests("test boots ... ignored, needs a hypervisor")))
    assert "Every test step succeeded; 1 test was ignored." in one.summary, one


def a_failed_step_and_a_skipped_one_are_named_with_nothing_ignored():
    """Case 7. Catches a report that reads only the log, which would say none was ignored."""
    result = report(FAILING_CORE_UNIT + MEMBRANE_UNIT, outcomes=("failure", "skipped"))
    assert result.code == 0, result
    assert "`tests` failed" in result.summary, result
    assert "`workspace guards` never ran" in result.summary, result
    assert COVERS in result.summary, result
    assert NONE not in result.summary, result
    assert "Every test step succeeded" not in result.summary, result
    assert BUILD + GUARDS.split() not in result.calls, result


def both_steps_skipped_with_no_log_is_not_a_crash():
    """Case 8. A red clippy skips both test steps, and the log file never exists."""
    result = report(None, outcomes=("skipped", "skipped"))
    assert result.code == 0, result
    assert "`tests` never ran" in result.summary, result
    assert "`workspace guards` never ran" in result.summary, result
    assert COVERS in result.summary, result
    assert NONE not in result.summary, result
    assert result.calls == [], result


def a_successful_step_whose_output_is_not_in_the_log_is_refused():
    """Case 9. Catches a tee path the report does not read."""
    missing = report(None)
    assert missing.code == 1, missing
    assert "cargo-test.log does not exist" in missing.stderr, missing
    assert TEE_HINT in missing.stderr, missing

    no_running = report(["   Compiling plasmosome-core v0.1.0 (/w/crates/core)",
                         "    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.31s"])
    assert no_running.code == 1, no_running
    assert TEE_HINT in no_running.stderr, no_running

    no_guards = report(tests_log())
    assert no_guards.code == 1, no_guards
    assert "`workspace guards` succeeded" in no_guards.stderr, no_guards
    assert TEE_HINT in no_guards.stderr, no_guards


def an_executable_or_library_missing_from_the_map_is_refused():
    """Case 10. Catches a map built with other package arguments than the step used."""
    log = log_of(core_cli=CORE_CLI + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-0dd0")
                 + tests("test x ... ok"))
    result = report(log)
    assert result.code == 1, result
    assert f"daemon_cli-0dd0 (tests/daemon_cli.rs) is in the log, but no step's cargo listing has it; {ARGS_HINT}" \
        in result.stderr, result

    log = CLEAN + doctests("plasmid_sdk") + tests()
    library = report(log)
    assert library.code == 1, library
    assert f"plasmid_sdk is in the log, but no step's cargo listing has it; {ARGS_HINT}" in library.stderr, library


def a_result_count_that_disagrees_with_the_listed_lines_is_refused():
    """Case 11. Catches a libtest format change that would silently drop ignored tests."""
    log = log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                 + tests("test boots ... ignored, needs a hypervisor", "test answers_usage ... ok", ignored=2))
    result = report(log)
    assert result.code == 1, result
    assert "daemon_cli-c0e2" in result.stderr, result
    assert "lists 1 ignored tests, but its result lines count 2" in result.stderr, result
    assert "read that step first" not in result.stderr, result

    phantom = log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                     + tests("test boots ... ignored, needs a hypervisor",
                             "test halts ... ignored, needs a hypervisor", ignored=1))
    more = report(phantom)
    assert more.code == 1, more
    assert "lists 2 ignored tests, but its result lines count 1" in more.stderr, more


def an_ignored_test_after_a_failing_binary_is_still_listed():
    """Case 12. The failing binary's block, error line included, does not swallow the next one."""
    log = (FAILING_CORE_UNIT
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests("test boots ... ignored, needs a hypervisor")
           + MEMBRANE_UNIT)
    result = report(log, outcomes=("failure", "skipped"))
    assert result.code == 0, result
    assert "| plasmosome-core | boots | needs a hypervisor |" in result.summary, result
    assert "`tests` failed" in result.summary, result
    assert COVERS in result.summary, result


def an_unknown_outcome_or_malformed_step_is_a_usage_error():
    """Case 13. Catches a mistyped step id, whose outcome GitHub gives as an empty string."""
    for outcome in ["", "succeeded"]:
        result = report(CLEAN, argv=lambda log, outcome=outcome: [
            log, f"tests={outcome}={TESTS}", f"workspace guards=success={GUARDS}"])
        assert result.code == 2, result
        assert result.calls == [], result
    for argv in [lambda log: [log], lambda log: [log, "tests"], lambda log: []]:
        result = report(CLEAN, argv=argv)
        assert result.code == 2, result


def colored_cargo_and_libtest_lines_still_parse():
    """Case 14. Catches a parser that a CARGO_TERM_COLOR=always or a colored harness would blind."""
    log = log_of(core_cli=[f"{ESC}[1m{ESC}[92m     Running{ESC}[0m tests/daemon_cli.rs "
                           "(target/debug/deps/daemon_cli-c0e2)", ""]
                 + tests(f"test boots ... {ESC}[33mignored{ESC}[0m, needs a hypervisor", ignored=1))
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | boots | needs a hypervisor |" in result.summary, result


def a_doctest_block_with_two_result_lines_sums_them():
    """Case 15. Merged and standalone doctests each print a result line in one Doc-tests block."""
    log = log_of(core_docs=doctests("plasmosome_core") + [
        "running 2 tests",
        "test crates/core/src/lib.rs - parse (line 7) ... ignored",
        "test crates/core/src/lib.rs - Kind (line 30) ... ok",
        "",
        result_line(1, 0, 1),
        "",
        "running 1 test",
        "test crates/core/src/lib.rs - Kind::from (line 44) ... ignored",
        "",
        result_line(0, 0, 1),
        "",
        "all doctests ran in 0.31s; merged doctests compilation took 0.29s",
        "",
    ])
    result = report(log)
    assert result.code == 0, result
    assert "Every test step succeeded; 2 tests were ignored." in result.summary, result


def a_block_with_no_result_line_is_refused_only_when_its_step_succeeded():
    """Case 16. Catches a result line the parser no longer recognizes, which would skip the count."""
    cut = log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                 + ["running 1 test", "test boots ... ignored, needs a hypervisor", ""])
    refused = report(cut)
    assert refused.code == 1, refused
    assert "daemon_cli-c0e2" in refused.stderr, refused

    crashed = report(cut, outcomes=("failure", "success"))
    assert crashed.code == 0, crashed
    assert "| plasmosome-core | boots | needs a hypervisor |" in crashed.summary, crashed
    assert "`tests` failed" in crashed.summary, crashed


def a_failed_listing_is_fatal_only_for_a_step_that_succeeded():
    """Case 17. Catches a report that vouches for a step whose executables it could not list."""
    refused = report(CLEAN, builds={TESTS: (101, TESTS_BUILD), GUARDS: (0, GUARDS_BUILD)})
    assert refused.code == 1, refused
    assert "tests" in refused.stderr, refused

    partial = TESTS_BUILD[:4]
    named = report(FAILING_CORE_UNIT, outcomes=("failure", "skipped"),
                   builds={TESTS: (101, partial), GUARDS: (0, GUARDS_BUILD)})
    assert named.code == 0, named
    assert "could not list the test executables of `tests`" in named.summary, named
    assert "`tests` failed" in named.summary, named

    metadata = report(CLEAN, metadata=(101, ""))
    assert metadata.code == 1, metadata
    assert "metadata" in metadata.stderr, metadata


def each_step_is_listed_with_its_own_package_arguments():
    """Case 18. Catches one --workspace listing, which misses every guards executable."""
    result = report(CLEAN)
    assert result.code == 0, result
    expected = [BUILD + TESTS.split(), BUILD + GUARDS.split(), METADATA_COMMAND]
    assert sorted(result.calls) == sorted(expected), result


def the_summary_is_appended_and_printed_under_the_job_name():
    """Case 19. Catches overwriting the step summary, or a heading that does not name the job."""
    result = report(CLEAN, job="macos")
    assert result.summary.startswith(EARLIER), result
    written = result.summary[len(EARLIER):]
    assert written == result.stdout, result
    heading = written.splitlines()[0]
    assert heading.startswith("## ") and "macos" in heading, result


def a_name_containing_ignored_is_not_an_ignored_test():
    """Case 20. Catches matching the word anywhere in the line."""
    log = log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                 + tests("test a_volume_mounted_with_ownership_ignored_is_refused ... ok"))
    result = report(log)
    assert result.code == 0, result
    assert NONE in result.summary, result


def a_pipe_in_a_reason_does_not_break_the_table():
    """Case 21. Catches a reason that splits its row into extra cells."""
    log = log_of(core_cli=running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                 + tests("test boots ... ignored, needs a guest | or a host"))
    result = report(log)
    assert result.code == 0, result
    assert f"| plasmosome-core | boots | needs a guest {BACKSLASH}| or a host |" in result.summary, result


def an_ignored_line_before_any_running_line_is_refused():
    """Case 22. Catches dropping an ignored test that no block can own."""
    log = ["test boots ... ignored, needs a hypervisor", ""] + CLEAN
    result = report(log)
    assert result.code == 1, result
    assert "before any Running line" in result.stderr, result


def unreadable_cargo_output_is_refused_for_a_step_that_succeeded():
    """Case 23. Catches reading unparseable JSON as an empty listing."""
    metadata = report(CLEAN, metadata=(0, "not json"))
    assert metadata.code == 1, metadata
    assert "unreadable" in metadata.stderr, metadata

    build = report(CLEAN, builds={TESTS: (0, ["not json"]), GUARDS: (0, GUARDS_BUILD)})
    assert build.code == 1, build
    assert "unreadable" in build.stderr, build
    assert "`tests`" in build.stderr, build


def a_metadata_failure_after_a_failed_compile_is_named_not_fatal():
    """Case 24. A broken manifest fails the test step and metadata alike; that is not the report's red."""
    log = ["error: failed to parse manifest at `/w/crates/core/Cargo.toml`"]
    result = report(log, outcomes=("failure", "skipped"), metadata=(101, ""))
    assert result.code == 0, result
    assert "`cargo metadata` failed" in result.summary, result
    assert "`tests` failed" in result.summary, result
    assert "`workspace guards` never ran" in result.summary, result


def an_executable_claimed_by_two_packages_is_refused():
    """Case 25. Catches silently picking one package for an executable two listings disagree on."""
    build = GUARDS_BUILD + [artifact(GUARDS_ID, "daemon_cli", "test", "/w/target/debug/deps/daemon_cli-c0e2")]
    result = report(CLEAN, builds={TESTS: (0, TESTS_BUILD), GUARDS: (0, build)})
    assert result.code == 1, result
    assert "more than one package" in result.stderr, result


def unparseable_step_arguments_are_a_usage_error():
    """Case 26. Catches a crash on an unbalanced quote in a step's package arguments."""
    result = report(CLEAN, argv=lambda log: [log, "tests=success=--workspace 'unterminated",
                                             f"workspace guards=success={GUARDS}"])
    assert result.code == 2, result
    assert result.calls == [], result


def a_refused_run_writes_its_refusal_to_the_step_summary():
    """Case 27. Catches a refusal that exits 1 but leaves the summary saying nothing was ignored."""
    result = report(tests_log())
    assert result.code == 1, result
    assert result.summary.startswith(EARLIER), result
    written = result.summary[len(EARLIER):]
    assert REFUSED in written, result
    assert "`workspace guards` succeeded" in written, result
    assert NONE not in written, result
    assert "Every test step succeeded" not in written, result
    assert written == result.stdout, result


def a_cancelled_step_is_named_as_cancelled():
    """Case 28. A superseded run cancels a step, and the report still runs under if: always()."""
    log = tests_log() + running("unittests src/lib.rs", "target/debug/deps/plasmosome_guards-6a71") + ["running 1 test"]
    result = report(log, outcomes=("success", "cancelled"))
    assert result.code == 0, result
    assert "`workspace guards` was cancelled" in result.summary, result
    assert "never ran" not in result.summary, result
    assert COVERS in result.summary, result
    assert NONE not in result.summary, result


def a_count_mismatch_in_a_failed_step_is_refused_and_points_at_that_step():
    """Case 29. Catches counting only ok result lines, or checking counts only when a step succeeded."""
    result = report(log_of(core_cli=FAILING_CORE_CLI_MISCOUNTED), outcomes=("failure", "success"))
    assert result.code == 1, result
    assert "daemon_cli-c0e2" in result.stderr, result
    assert "lists 1 ignored tests, but its result lines count 2" in result.stderr, result
    assert TESTS_FAILED_HINT in result.stderr, result
    assert TESTS_FAILED_HINT in result.summary, result


def a_block_two_listings_share_needs_a_result_line_only_when_both_steps_succeeded():
    """Case 30. The guards listing names plasmosome_core's library too. Catches all() turned into any()."""
    log = log_of(core_docs=doctests("plasmosome_core")
                 + ["running 1 test", "test crates/core/src/lib.rs - parse (line 7) ... ignored"])
    crashed = report(log, outcomes=("failure", "success"))
    assert crashed.code == 0, crashed
    assert "| plasmosome-core | crates/core/src/lib.rs - parse (line 7) | no reason given |" in crashed.summary, crashed
    assert "`tests` failed" in crashed.summary, crashed

    refused = report(log)
    assert refused.code == 1, refused
    assert "plasmosome_core has no `test result:` line" in refused.stderr, refused


def a_log_missing_whole_blocks_of_a_successful_step_is_refused():
    """Case 31. Catches a cut log, or report arguments wider than the step's, that ends between blocks."""
    cut = report(tests_log(membrane_unit=[], membrane_cli=[]) + guards_log())
    assert cut.code == 1, cut
    assert "plasmosome_membrane-3e31" in cut.stderr, cut
    assert "daemon_cli-3e32" in cut.stderr, cut
    assert TEE_HINT in cut.stderr, cut
    assert "daemon_cli-c0e2" not in cut.stderr, cut

    guards = report(log_of(roots=[]))
    assert guards.code == 1, guards
    assert "`workspace guards` succeeded" in guards.stderr, guards
    assert "workspace_roots-6a73" in guards.stderr, guards


def a_failing_guard_test_that_prints_a_nested_cargo_run_keeps_the_list():
    """Case 32. workspace_roots puts a nested cargo run's output in its panic message.

    Its Running line names either the real workspace_guards file again or one rebuilt elsewhere.
    Catches a red guard test turning the report red too, and losing the list for that run.
    """
    for nested, why in [(f"{NESTED}/workspace_guards-6a72", "appears a second time"),
                        (f"{NESTED}/workspace_guards-9f9f", "is in the log, but no step's cargo listing has it")]:
        log = log_of(core_cli=(running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                               + tests("test boots ... ignored, needs a hypervisor", "test answers_usage ... ok")),
                     roots=nested_failure(nested))
        result = report(log, outcomes=("success", "failure"))
        assert result.code == 0, result
        assert REFUSED not in result.summary, result
        assert "| plasmosome-core | boots | needs a hypervisor |" in result.summary, result
        assert "| plasmosome-guards | held_names_stay_held | needs a registry |" in result.summary, result
        assert "`workspace guards` failed" in result.summary, result
        assert COVERS in result.summary, result
        key = nested.rsplit("/", 1)[-1]
        assert f"{key} (tests/workspace_guards.rs) {why}" in result.summary, result
        assert "output from a failing test" in result.summary, result

        vouched = report(log)
        assert vouched.code == 1, vouched


CASES = [
    one_file_name_in_two_crates_lists_each_under_its_own_crate,
    a_doctest_is_listed_under_its_own_crate_with_no_reason_given,
    a_reason_is_kept_verbatim,
    package_names_come_from_metadata_not_from_the_id,
    relative_and_absolute_running_paths_both_map,
    the_status_sentence_says_none_or_counts,
    a_failed_step_and_a_skipped_one_are_named_with_nothing_ignored,
    both_steps_skipped_with_no_log_is_not_a_crash,
    a_successful_step_whose_output_is_not_in_the_log_is_refused,
    an_executable_or_library_missing_from_the_map_is_refused,
    a_result_count_that_disagrees_with_the_listed_lines_is_refused,
    an_ignored_test_after_a_failing_binary_is_still_listed,
    an_unknown_outcome_or_malformed_step_is_a_usage_error,
    colored_cargo_and_libtest_lines_still_parse,
    a_doctest_block_with_two_result_lines_sums_them,
    a_block_with_no_result_line_is_refused_only_when_its_step_succeeded,
    a_failed_listing_is_fatal_only_for_a_step_that_succeeded,
    each_step_is_listed_with_its_own_package_arguments,
    the_summary_is_appended_and_printed_under_the_job_name,
    a_name_containing_ignored_is_not_an_ignored_test,
    a_pipe_in_a_reason_does_not_break_the_table,
    an_ignored_line_before_any_running_line_is_refused,
    unreadable_cargo_output_is_refused_for_a_step_that_succeeded,
    a_metadata_failure_after_a_failed_compile_is_named_not_fatal,
    an_executable_claimed_by_two_packages_is_refused,
    unparseable_step_arguments_are_a_usage_error,
    a_refused_run_writes_its_refusal_to_the_step_summary,
    a_cancelled_step_is_named_as_cancelled,
    a_count_mismatch_in_a_failed_step_is_refused_and_points_at_that_step,
    a_block_two_listings_share_needs_a_result_line_only_when_both_steps_succeeded,
    a_log_missing_whole_blocks_of_a_successful_step_is_refused,
    a_failing_guard_test_that_prints_a_nested_cargo_run_keeps_the_list,
]


def main():
    failed = []
    for case in CASES:
        try:
            case()
        except Exception:
            failed.append(case.__name__)
            print(f"FAIL: {case.__name__}", file=sys.stderr)
            traceback.print_exc()
    if failed:
        sys.exit(f"{len(failed)} of {len(CASES)} ignored-test report cases failed: {', '.join(failed)}")
    print(f"PASS: {len(CASES)} ignored-test report cases: crates by executable, doctests, verbatim "
          "reasons, metadata names, status sentences, failed, cancelled and skipped steps, refusals and "
          "their hints, cut logs, nested cargo output, colored lines")


if __name__ == "__main__":
    main()
