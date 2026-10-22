#![deny(clippy::exit)]

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use plasmosome_tools::bench_medians::{ESTIMATES_LIMIT, run};

const NAMES: [&str; 8] = [
    "attach_detach",
    "ledger_replay/10",
    "ledger_replay/100",
    "ledger_replay/1000",
    "manifest_parse",
    "reconciler_step",
    "session_log_append",
    "session_log_read",
];

const MEDIANS: [&str; 8] = [
    "81234.5", "812.25", "8123", "81234.125", "0.75", "1.0", "65.000001", "1e3",
];

const LISTED: &str = "## Kernel benchmark medians

FakeBackend timings exclude OS enforcement.

- `attach_detach` 81234.500000 ns
- `ledger_replay/10` 812.250000 ns
- `ledger_replay/100` 8123.000000 ns
- `ledger_replay/1000` 81234.125000 ns
- `manifest_parse` 0.750000 ns
- `reconciler_step` 1.000000 ns
- `session_log_append` 65.000001 ns
- `session_log_read` 1000.000000 ns
";

struct Outcome {
    code: u8,
    out: String,
    err: String,
}

fn estimates(median: &str) -> String {
    format!(
        "{{\"mean\":{{\"point_estimate\":2.5,\"standard_error\":0.1}},\"median\":{{\"confidence_interval\":{{\"confidence_level\":0.95,\"lower_bound\":1.0,\"upper_bound\":4.0}},\"point_estimate\":{median},\"standard_error\":0.2}},\"slope\":null}}"
    )
}

fn write_estimates(root: &Path, name: &str, text: &str) {
    let directory = root.join(name).join("new");
    std::fs::create_dir_all(&directory).expect("the benchmark directory is created");
    std::fs::write(directory.join("estimates.json"), text).expect("estimates.json is written");
}

fn criterion() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    for (name, median) in NAMES.iter().zip(MEDIANS) {
        write_estimates(directory.path(), name, &estimates(median));
    }
    directory
}

fn run_on(root: &Path, summary: Option<&Path>) -> Outcome {
    run_with(&[root.as_os_str().to_owned()], summary)
}

fn run_with(args: &[OsString], summary: Option<&Path>) -> Outcome {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(args, summary, &mut out, &mut err);
    Outcome {
        code,
        out: String::from_utf8(out).expect("stdout is UTF-8"),
        err: String::from_utf8(err).expect("stderr is UTF-8"),
    }
}

fn assert_one_line_naming(outcome: &Outcome, code: u8, named: &str) {
    assert_eq!(outcome.code, code, "{}", outcome.err);
    assert!(
        outcome.err.starts_with("bench-medians: ") && outcome.err.ends_with('\n'),
        "{:?}",
        outcome.err
    );
    assert_eq!(outcome.err.lines().count(), 1, "{:?}", outcome.err);
    assert!(outcome.err.contains(named), "{:?}", outcome.err);
}

#[test]
fn the_eight_medians_are_listed_in_order() {
    let criterion = criterion();
    let summary = criterion.path().join("summary.md");

    let outcome = run_on(criterion.path(), Some(&summary));

    assert_eq!(outcome.code, 0, "{}", outcome.err);
    assert_eq!(outcome.out, LISTED);
    assert_eq!(outcome.err, "");
    assert_eq!(std::fs::read_to_string(&summary).unwrap(), LISTED);
}

#[test]
fn a_median_prints_as_python_prints_it() {
    for (median, printed) in [
        ("1234.5678905", "1234.567890"),
        ("1234.56789051", "1234.567891"),
        ("0.0078125", "0.007812"),
        ("0.0234375", "0.023438"),
        ("5e-7", "0.000000"),
        ("1234", "1234.000000"),
        ("-0.0", "-0.000000"),
        ("1e30", "1000000000000000019884624838656.000000"),
    ] {
        let criterion = criterion();
        write_estimates(criterion.path(), "manifest_parse", &estimates(median));
        let summary = criterion.path().join("summary.md");

        let outcome = run_on(criterion.path(), Some(&summary));

        assert_eq!(outcome.code, 0, "{median}: {}", outcome.err);
        let line = format!("- `manifest_parse` {printed} ns\n");
        assert!(outcome.out.contains(&line), "{median}: {}", outcome.out);
    }
}

#[test]
fn a_missing_benchmark_is_named_and_nothing_is_written() {
    let criterion = criterion();
    std::fs::remove_dir_all(criterion.path().join("ledger_replay/100")).unwrap();
    let summary = criterion.path().join("summary.md");

    let outcome = run_on(criterion.path(), Some(&summary));

    assert_one_line_naming(&outcome, 1, "ledger_replay/100/new/estimates.json");
    assert!(outcome.err.contains("cannot read"), "{:?}", outcome.err);
    assert_eq!(outcome.out, "");
    assert!(!summary.exists(), "a failed run wrote the summary");
}

#[test]
fn a_median_that_is_missing_or_not_a_number_is_named() {
    for text in [
        "{\"mean\":{\"point_estimate\":2.5}}".to_string(),
        "{\"median\":2.5}".to_string(),
        "[2.5]".to_string(),
        estimates("null"),
        estimates("\"2.5\""),
        estimates("true"),
    ] {
        let criterion = criterion();
        write_estimates(criterion.path(), "session_log_read", &text);

        let outcome = run_on(criterion.path(), Some(&criterion.path().join("summary.md")));

        assert_one_line_naming(&outcome, 1, "session_log_read/new/estimates.json");
        assert!(
            outcome.err.contains("has no number at median.point_estimate"),
            "{text}: {:?}",
            outcome.err
        );
        assert_eq!(outcome.out, "");
    }
}

#[test]
fn a_file_that_is_not_json_is_named() {
    for text in ["", "{\"median\":", "NaN", "{\"median\":{\"point_estimate\":NaN}}"] {
        let criterion = criterion();
        write_estimates(criterion.path(), "attach_detach", text);

        let outcome = run_on(criterion.path(), None);

        assert_one_line_naming(&outcome, 1, "attach_detach/new/estimates.json");
        assert!(outcome.err.contains("is not JSON"), "{text}: {:?}", outcome.err);
        assert_eq!(outcome.out, "");
    }
}

fn padded_to(length: u64) -> String {
    let text = estimates("7.5");
    let padding = usize::try_from(length).unwrap() - text.len();
    format!("{text}{}", " ".repeat(padding))
}

#[test]
fn a_file_at_the_bound_is_read() {
    let criterion = criterion();
    write_estimates(criterion.path(), "reconciler_step", &padded_to(ESTIMATES_LIMIT));

    let outcome = run_on(criterion.path(), Some(&criterion.path().join("summary.md")));

    assert_eq!(outcome.code, 0, "{}", outcome.err);
    assert!(outcome.out.contains("- `reconciler_step` 7.500000 ns\n"));
}

#[test]
fn a_file_one_byte_over_the_bound_is_refused() {
    let criterion = criterion();
    write_estimates(
        criterion.path(),
        "reconciler_step",
        &padded_to(ESTIMATES_LIMIT + 1),
    );

    let outcome = run_on(criterion.path(), Some(&criterion.path().join("summary.md")));

    assert_one_line_naming(&outcome, 1, "reconciler_step/new/estimates.json");
    assert!(
        outcome.err.contains("is over 1048576 bytes"),
        "{:?}",
        outcome.err
    );
    assert_eq!(outcome.out, "");
}

#[test]
fn the_summary_is_appended_not_truncated() {
    let criterion = criterion();
    let summary = criterion.path().join("summary.md");
    std::fs::write(&summary, "an earlier step's summary\n").unwrap();

    let first = run_on(criterion.path(), Some(&summary));
    let second = run_on(criterion.path(), Some(&summary));

    assert_eq!((first.code, second.code), (0, 0), "{}", first.err);
    assert_eq!(
        std::fs::read_to_string(&summary).unwrap(),
        format!("an earlier step's summary\n{LISTED}{LISTED}")
    );
}

#[test]
fn an_unset_summary_prints_the_medians_then_fails() {
    let criterion = criterion();

    let outcome = run_on(criterion.path(), None);

    assert_one_line_naming(&outcome, 1, "GITHUB_STEP_SUMMARY is not set");
    assert_eq!(outcome.out, LISTED);
}

#[test]
fn a_summary_that_cannot_be_opened_fails_after_printing() {
    let criterion = criterion();

    let outcome = run_on(criterion.path(), Some(criterion.path()));

    assert_one_line_naming(&outcome, 1, "cannot append to");
    assert_eq!(outcome.out, LISTED);
}

#[test]
fn anything_but_one_argument_is_a_usage_error() {
    let criterion = criterion();
    let summary = criterion.path().join("summary.md");
    let root = criterion.path().as_os_str().to_owned();

    for args in [vec![], vec![root.clone(), root]] {
        let outcome = run_with(&args, Some(&summary));

        assert_one_line_naming(&outcome, 2, "expected one argument");
        assert_eq!(outcome.out, "");
    }
    assert!(!summary.exists());
}

#[test]
fn the_binary_appends_to_the_file_github_step_summary_names() {
    let criterion = criterion();
    let summary = criterion.path().join("summary.md");
    let binary = env!("CARGO_BIN_EXE_bench-medians");

    let named = Command::new(binary)
        .arg(criterion.path())
        .env("GITHUB_STEP_SUMMARY", &summary)
        .output()
        .expect("bench-medians runs");
    let unset = Command::new(binary)
        .arg(criterion.path())
        .env_remove("GITHUB_STEP_SUMMARY")
        .output()
        .expect("bench-medians runs");

    assert_eq!(named.status.code(), Some(0), "{named:?}");
    assert_eq!(String::from_utf8(named.stdout).unwrap(), LISTED);
    assert_eq!(std::fs::read_to_string(&summary).unwrap(), LISTED);
    assert_eq!(unset.status.code(), Some(1), "{unset:?}");
    assert_eq!(
        String::from_utf8(unset.stderr).unwrap(),
        "bench-medians: GITHUB_STEP_SUMMARY is not set\n"
    );
}
