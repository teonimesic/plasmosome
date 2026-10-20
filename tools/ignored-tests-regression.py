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
GUARDS_RUN = (running("tests/workspace_guards.rs", "target/debug/deps/workspace_guards-6a72")
              + tests("test ci_matrix_matches_workspace_members ... ok"))
CLEAN = CORE_UNIT + CORE_CLI + MEMBRANE_UNIT + MEMBRANE_CLI + doctests("plasmosome_core") + tests() + GUARDS_RUN

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
    log = (CORE_UNIT
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests("test boots ... ignored, needs a hypervisor", "test answers_usage ... ok")
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-3e32")
           + tests("test boots ... ignored, needs a hypervisor")
           + GUARDS_RUN)
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | boots | needs a hypervisor |" in result.summary, result
    assert "| plasmosome-membrane | boots | needs a hypervisor |" in result.summary, result


def a_doctest_is_listed_under_its_own_crate_with_no_reason_given():
    """Case 2. Catches a doctest given the previous binary's crate, and dropping a reasonless line."""
    log = (CORE_UNIT + MEMBRANE_UNIT
           + doctests("plasmosome_core")
           + tests("test crates/core/src/lib.rs - parse (line 7) ... ignored")
           + GUARDS_RUN)
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | crates/core/src/lib.rs - parse (line 7) | no reason given |" in result.summary, result
    assert "plasmosome-membrane |" not in result.summary, result


def a_reason_is_kept_verbatim():
    """Case 3. Catches splitting the reason on its commas or dots."""
    reason = "needs a booted cell, a signed kernel... and, later, a network"
    log = (CORE_UNIT
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests(f"test boots ... ignored, {reason}")
           + GUARDS_RUN)
    result = report(log)
    assert result.code == 0, result
    assert f"| plasmosome-core | boots | {reason} |" in result.summary, result


def package_names_come_from_metadata_not_from_the_id():
    """Case 4. CORE's id names a directory, MEMBRANE's carries #name@version. Catches splitting the id."""
    log = (running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests("test boots ... ignored, needs a hypervisor")
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-3e32")
           + tests("test boots ... ignored, needs a hypervisor")
           + GUARDS_RUN)
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | boots |" in result.summary, result
    assert "| plasmosome-membrane | boots |" in result.summary, result
    assert "| core |" not in result.summary, result

    stranger = "path+file:///w/crates/stranger#0.1.0"
    build = TESTS_BUILD + [artifact(stranger, "stranger", "test", "/w/target/debug/deps/stranger-5a51")]
    log = CORE_UNIT + running("tests/stranger.rs", "target/debug/deps/stranger-5a51") + tests("test x ... ok") + GUARDS_RUN
    unnamed = report(log, builds={TESTS: (0, build), GUARDS: (0, GUARDS_BUILD)})
    assert unnamed.code == 1, unnamed
    assert "stranger" in unnamed.stderr, unnamed


def relative_and_absolute_running_paths_both_map():
    """Case 5. Catches matching the whole path, which cargo prints either way."""
    log = (running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests("test boots ... ignored, needs a hypervisor")
           + running("tests/daemon_cli.rs", "/w/target/debug/deps/daemon_cli-3e32")
           + tests("test boots ... ignored, needs a hypervisor")
           + GUARDS_RUN)
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

    two = report(CORE_UNIT
                 + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                 + tests("test boots ... ignored, needs a hypervisor", "test halts ... ignored, needs a hypervisor")
                 + GUARDS_RUN)
    assert two.code == 0, two
    assert "Every test step succeeded; 2 tests were ignored." in two.summary, two
    assert NONE not in two.summary, two
    assert HEADER in two.summary, two

    one = report(CORE_UNIT
                 + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
                 + tests("test boots ... ignored, needs a hypervisor")
                 + GUARDS_RUN)
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

    no_running = report(["   Compiling plasmosome-core v0.1.0 (/w/crates/core)",
                         "    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.31s"])
    assert no_running.code == 1, no_running

    no_guards = report(CORE_UNIT + CORE_CLI + MEMBRANE_UNIT + MEMBRANE_CLI)
    assert no_guards.code == 1, no_guards
    assert "workspace guards" in no_guards.stderr, no_guards


def an_executable_or_library_missing_from_the_map_is_refused():
    """Case 10. Catches a map built with other package arguments than the step used."""
    log = CORE_UNIT + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-0dd0") + tests("test x ... ok") + GUARDS_RUN
    result = report(log)
    assert result.code == 1, result
    assert "daemon_cli-0dd0" in result.stderr, result

    log = CORE_UNIT + GUARDS_RUN + doctests("plasmid_sdk") + tests()
    library = report(log)
    assert library.code == 1, library
    assert "plasmid_sdk" in library.stderr, library


def a_result_count_that_disagrees_with_the_listed_lines_is_refused():
    """Case 11. Catches a libtest format change that would silently drop ignored tests."""
    log = (CORE_UNIT
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests("test boots ... ignored, needs a hypervisor", "test answers_usage ... ok", ignored=2)
           + GUARDS_RUN)
    result = report(log)
    assert result.code == 1, result
    assert "daemon_cli-c0e2" in result.stderr, result


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
    log = ([f"{ESC}[1m{ESC}[92m     Running{ESC}[0m tests/daemon_cli.rs (target/debug/deps/daemon_cli-c0e2)", ""]
           + tests(f"test boots ... {ESC}[33mignored{ESC}[0m, needs a hypervisor", ignored=1)
           + GUARDS_RUN)
    result = report(log)
    assert result.code == 0, result
    assert "| plasmosome-core | boots | needs a hypervisor |" in result.summary, result


def a_doctest_block_with_two_result_lines_sums_them():
    """Case 15. Merged and standalone doctests each print a result line in one Doc-tests block."""
    log = (CORE_UNIT + doctests("plasmosome_core") + [
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
    ] + GUARDS_RUN)
    result = report(log)
    assert result.code == 0, result
    assert "Every test step succeeded; 2 tests were ignored." in result.summary, result


def a_block_with_no_result_line_is_refused_only_when_its_step_succeeded():
    """Case 16. Catches a result line the parser no longer recognizes, which would skip the count."""
    cut = (CORE_UNIT
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + ["running 1 test", "test boots ... ignored, needs a hypervisor", ""]
           + GUARDS_RUN)
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
    log = (CORE_UNIT
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests("test a_volume_mounted_with_ownership_ignored_is_refused ... ok")
           + GUARDS_RUN)
    result = report(log)
    assert result.code == 0, result
    assert NONE in result.summary, result


def a_pipe_in_a_reason_does_not_break_the_table():
    """Case 21. Catches a reason that splits its row into extra cells."""
    log = (CORE_UNIT
           + running("tests/daemon_cli.rs", "target/debug/deps/daemon_cli-c0e2")
           + tests("test boots ... ignored, needs a guest | or a host")
           + GUARDS_RUN)
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
          "reasons, metadata names, status sentences, failed and skipped steps, refusals, colored lines")


if __name__ == "__main__":
    main()
