#![deny(clippy::exit)]

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use plasmosome_tools::bench_medians::{ESTIMATES_LIMIT, run};

const MEDIANS: [(&str, &str); 8] = [
    ("attach_detach", "81234.5"),
    ("ledger_replay/10", "812.25"),
    ("ledger_replay/100", "8123"),
    ("ledger_replay/1000", "81234.125"),
    ("manifest_parse", "0.75"),
    ("reconciler_step", "1.0"),
    ("session_log_append", "65.000001"),
    ("session_log_read", "1e3"),
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
    for (name, median) in MEDIANS {
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
        (
            "123456789012345678901234567890",
            "123456789012345677877719597056.000000",
        ),
        ("9007199254740993", "9007199254740992.000000"),
    ] {
        assert_eq!(printed_for(median), printed, "{median}");
    }
}

/// Each median is the text serde_json writes for a double, as criterion writes it, and that double
/// lies beside a tie at the sixth decimal. serde_json's own conversion, without its
/// `float_roundtrip` feature, reads each of these one double away, on the tie's other side.
#[test]
fn a_median_beside_a_sixth_decimal_tie_prints_as_python_prints_it() {
    let wrong: Vec<String> = [
        ("1198.8574505000001", "1198.857451"),
        ("22717.054650500002", "22717.054651"),
        ("97027.32520050001", "97027.325201"),
        ("29191.989560500002", "29191.989561"),
        ("1927.6589004999998", "1927.658900"),
        ("260176.12600050002", "260176.126001"),
        ("941874.4580005001", "941874.458001"),
        ("32365.932700499998", "32365.932700"),
        ("14173757.850406501", "14173757.850407"),
        ("927669.9315965001", "927669.931597"),
        ("9198282.690333499", "9198282.690333"),
    ]
    .into_iter()
    .filter_map(|(median, printed)| {
        let got = printed_for(median);
        (got != printed).then(|| format!("{median} printed {got}, not {printed}"))
    })
    .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Python reads a number with no fraction or exponent as an `int`, which has no negative zero, and
/// every other number as a float, which keeps the sign.
#[test]
fn only_the_integer_zero_loses_its_sign() {
    for (median, printed) in [
        ("-0", "0.000000"),
        ("0", "0.000000"),
        ("-0e0", "-0.000000"),
        ("-0E0", "-0.000000"),
        ("-0E+0", "-0.000000"),
        ("-0.0e0", "-0.000000"),
        ("-1e-400", "-0.000000"),
    ] {
        assert_eq!(printed_for(median), printed, "{median}");
    }
}

#[test]
fn a_repeated_key_is_read_at_its_last_value() {
    for (text, printed) in [
        (
            "{\"median\":{\"point_estimate\":\"1.5\"},\"median\":{\"point_estimate\":2.5}}",
            "2.500000",
        ),
        (
            "{\"median\":{\"point_estimate\":1.5,\"point_estimate\":0.25}}",
            "0.250000",
        ),
    ] {
        let criterion = criterion();
        write_estimates(criterion.path(), "manifest_parse", text);

        assert_eq!(printed_of(criterion.path()), printed, "{text}");
    }
}

#[test]
fn a_key_with_an_escape_is_read() {
    let criterion = criterion();
    write_estimates(
        criterion.path(),
        "manifest_parse",
        "{\"note\\/1\":1,\"median\":{\"point_estimate\":2.5}}",
    );

    assert_eq!(printed_of(criterion.path()), "2.500000");
}

/// What the tool prints as `manifest_parse`'s median when its `estimates.json` holds `median`.
fn printed_for(median: &str) -> String {
    let criterion = criterion();
    write_estimates(criterion.path(), "manifest_parse", &estimates(median));
    printed_of(criterion.path())
}

/// What the tool prints as `manifest_parse`'s median from `root`, or how it failed.
fn printed_of(root: &Path) -> String {
    let outcome = run_on(root, Some(&root.join("summary.md")));
    if outcome.code != 0 {
        return format!("exit {}: {}", outcome.code, outcome.err);
    }
    outcome
        .out
        .lines()
        .find_map(|line| {
            line.strip_prefix("- `manifest_parse` ")?
                .strip_suffix(" ns")
        })
        .unwrap_or("no manifest_parse line")
        .to_owned()
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
        estimates("false"),
        estimates("[2.5]"),
        estimates("{\"point_estimate\":2.5}"),
    ] {
        let criterion = criterion();
        write_estimates(criterion.path(), "session_log_read", &text);

        let outcome = run_on(criterion.path(), Some(&criterion.path().join("summary.md")));

        assert_one_line_naming(&outcome, 1, "session_log_read/new/estimates.json");
        assert!(
            outcome
                .err
                .contains("has no number at median.point_estimate"),
            "{text}: {:?}",
            outcome.err
        );
        assert_eq!(outcome.out, "");
    }
}

#[test]
fn a_file_that_is_not_json_is_named() {
    for text in [
        String::new(),
        "{\"median\":".to_string(),
        "NaN".to_string(),
        "{\"median\":{\"point_estimate\":NaN}}".to_string(),
        estimates("Infinity"),
        estimates("1e400"),
        format!("\x0c{}", estimates("7.5")),
        format!(
            "{{\"median\":{{\"point_estimate\":1.5,\"deep\":{}{}}}}}",
            "[".repeat(129),
            "]".repeat(129)
        ),
    ] {
        let criterion = criterion();
        write_estimates(criterion.path(), "attach_detach", &text);

        let outcome = run_on(criterion.path(), None);

        assert_one_line_naming(&outcome, 1, "attach_detach/new/estimates.json");
        assert!(
            outcome.err.contains("is not JSON"),
            "{text}: {:?}",
            outcome.err
        );
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
    write_estimates(
        criterion.path(),
        "reconciler_step",
        &padded_to(ESTIMATES_LIMIT),
    );

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

struct ClosedPipe;

impl std::io::Write for ClosedPipe {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn a_stdout_that_cannot_be_written_fails_before_the_summary() {
    let criterion = criterion();
    let summary = criterion.path().join("summary.md");
    let mut err = Vec::new();

    let code = run(
        &[criterion.path().as_os_str().to_owned()],
        Some(&summary),
        &mut ClosedPipe,
        &mut err,
    );

    let outcome = Outcome {
        code,
        out: String::new(),
        err: String::from_utf8(err).unwrap(),
    };
    assert_one_line_naming(&outcome, 1, "cannot write to stdout");
    assert!(
        !summary.exists(),
        "the summary was written after stdout failed"
    );
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
