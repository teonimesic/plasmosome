use std::collections::BTreeMap;

use plasmosome_lanes::gate::Verdict;
use plasmosome_lanes::results::{Expect, Record, RowSpec, RunSpec, render, row_verdict};
use serde_json::json;

fn run(name: &str, expect: Expect) -> RunSpec {
    RunSpec {
        run: name.to_string(),
        what: format!("what {name} is"),
        expect,
    }
}

fn row(id: &str, runs: Vec<RunSpec>) -> RowSpec {
    RowSpec {
        id: id.to_string(),
        claim: format!("the claim of {id}"),
        runs,
    }
}

fn record(name: &str, verdicts: &[(&str, Verdict)]) -> Record {
    Record {
        run: name.to_string(),
        configuration: json!({"shares": [{"tag": "sw", "read_only": true}]}),
        digests: BTreeMap::from([("initramfs".to_string(), format!("digest-of-{name}"))]),
        verdicts: verdicts
            .iter()
            .map(|(row, verdict)| (row.to_string(), verdict.clone()))
            .collect(),
        observations: BTreeMap::new(),
        notes: vec![format!("note of {name}")],
        lines: vec![format!("LANE-RESULT line of {name}")],
    }
}

fn m1() -> RowSpec {
    row(
        "M1",
        vec![run("real", Expect::Pass), run("simplified", Expect::Fail)],
    )
}

#[test]
fn a_row_passes_when_its_configuration_passes_and_every_mutant_fails() {
    let records = [
        record("real", &[("M1", Verdict::Pass)]),
        record(
            "simplified",
            &[("M1", Verdict::Fail("1000:1000".to_string()))],
        ),
    ];

    assert_eq!(row_verdict(&m1(), &records), Verdict::Pass);
}

#[test]
fn a_row_fails_when_a_mutant_passes_or_its_configuration_fails() {
    let mutant_passed = [
        record("real", &[("M1", Verdict::Pass)]),
        record("simplified", &[("M1", Verdict::Pass)]),
    ];
    assert!(
        matches!(row_verdict(&m1(), &mutant_passed), Verdict::Fail(reason) if reason.contains("simplified"))
    );
    let real_failed = [
        record("real", &[("M1", Verdict::Fail("501:20".to_string()))]),
        record(
            "simplified",
            &[("M1", Verdict::Fail("1000:1000".to_string()))],
        ),
    ];
    assert!(
        matches!(row_verdict(&m1(), &real_failed), Verdict::Fail(reason) if reason.contains("real"))
    );
}

#[test]
fn a_row_with_a_missing_run_or_verdict_is_not_run() {
    let missing = [record("real", &[("M1", Verdict::Pass)])];
    assert!(
        matches!(row_verdict(&m1(), &missing), Verdict::NotRun(reason) if reason.contains("simplified"))
    );
    let unjudged = [
        record("real", &[("M1", Verdict::Pass)]),
        record("simplified", &[("M2", Verdict::Pass)]),
    ];
    assert!(matches!(row_verdict(&m1(), &unjudged), Verdict::NotRun(_)));
    let not_run = [
        record("real", &[("M1", Verdict::NotRun("no console".to_string()))]),
        record(
            "simplified",
            &[("M1", Verdict::Fail("1000:1000".to_string()))],
        ),
    ];
    assert!(matches!(row_verdict(&m1(), &not_run), Verdict::NotRun(_)));
}

#[test]
fn an_observation_run_never_changes_a_rows_verdict() {
    let rows = row(
        "O1",
        vec![run("real", Expect::Pass), run("nowait", Expect::Observe)],
    );
    let records = [
        record("real", &[("O1", Verdict::Pass)]),
        record("nowait", &[("O1", Verdict::Fail("cached".to_string()))]),
    ];

    assert_eq!(row_verdict(&rows, &records), Verdict::Pass);
}

#[test]
fn a_run_missing_from_the_records_is_written_as_not_run() {
    let rows = [m1(), row("R1", vec![run("eighty", Expect::Pass)])];
    let records = [record("real", &[("M1", Verdict::Pass)])];

    let text = render("# Results\n\nPreamble.\n", &rows, &records);

    assert!(text.starts_with("# Results\n\nPreamble.\n"), "{text}");
    let row_lines: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("| M1 ") || line.starts_with("| R1 "))
        .collect();
    assert!(
        row_lines
            .iter()
            .any(|line| line.contains("simplified") && line.contains("not run")),
        "{text}"
    );
    assert!(
        row_lines
            .iter()
            .any(|line| line.contains("eighty") && line.contains("not run")),
        "{text}"
    );
    assert!(text.contains("### eighty"), "{text}");
    assert!(text.contains("### simplified"), "{text}");
    assert!(text.contains("LANE-RESULT line of real"), "{text}");
    assert!(text.contains("digest-of-real"), "{text}");
    assert!(text.contains("note of real"), "{text}");
    assert!(text.contains("\"read_only\": true"), "{text}");
}

#[test]
fn the_verdict_table_names_each_rows_verdict_and_each_runs_reason() {
    let records = [
        record(
            "real",
            &[("M1", Verdict::Fail("owned by 501:20".to_string()))],
        ),
        record(
            "simplified",
            &[("M1", Verdict::Fail("owned by 1000:1000".to_string()))],
        ),
    ];

    let text = render("", &[m1()], &records);

    assert!(text.contains("owned by 501:20"), "{text}");
    assert!(text.contains("owned by 1000:1000"), "{text}");
    assert!(
        text.lines()
            .any(|line| line.starts_with("| M1 |") && line.contains("**fail**")),
        "{text}"
    );
}

#[test]
fn a_pipe_in_a_reason_does_not_break_the_table() {
    let records = [
        record("real", &[("M1", Verdict::Fail("a | b".to_string()))]),
        record("simplified", &[("M1", Verdict::Fail("c".to_string()))]),
    ];

    let text = render("", &[m1()], &records);

    assert!(text.contains("a \\| b"), "{text}");
}

#[test]
fn observations_are_written_under_their_run() {
    let mut observed = record("real", &[("M1", Verdict::Pass)]);
    observed.observations.insert(
        "O1".to_string(),
        vec!["cafe NFC / NFD: one name".to_string()],
    );
    let records = [
        observed,
        record("simplified", &[("M1", Verdict::Fail("c".to_string()))]),
    ];

    let text = render("", &[m1()], &records);

    assert!(text.contains("cafe NFC / NFD: one name"), "{text}");
}
