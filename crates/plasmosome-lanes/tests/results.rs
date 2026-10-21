use std::collections::BTreeMap;

use plasmosome_lanes::gate::Verdict;
use plasmosome_lanes::results::{
    Expect, Record, RowSpec, RunSpec, one_commit, render, row_verdict,
};
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
        counts: BTreeMap::new(),
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
fn a_row_whose_mutant_did_not_run_is_not_run_and_never_caught() {
    let records = [
        record("real", &[("M1", Verdict::Pass)]),
        record(
            "simplified",
            &[("M1", Verdict::NotRun("the VM did not start".to_string()))],
        ),
    ];

    assert!(
        matches!(row_verdict(&m1(), &records), Verdict::NotRun(reason) if reason.contains("simplified") && reason.contains("the VM did not start"))
    );
    let text = render("", &[m1()], &records);
    assert!(
        text.lines().any(
            |line| line.starts_with("| M1 | the claim of M1 |") && line.contains("**not run**")
        ),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("| M1 | simplified |")
                && line.ends_with("| not run: the VM did not start |")),
        "{text}"
    );
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

#[test]
fn each_observation_row_is_tabled_after_the_verdicts_with_not_run_for_a_missing_record() {
    let names = row(
        "Names",
        vec![run("real", Expect::Observe), run("absent", Expect::Observe)],
    );
    let mut observed = record("real", &[("M1", Verdict::Pass)]);
    observed.observations.insert(
        "Names".to_string(),
        vec!["cafe NFC / NFD: one name".to_string(), "a | b".to_string()],
    );
    let records = [
        observed,
        record("simplified", &[("M1", Verdict::Fail("c".to_string()))]),
    ];

    let text = render("", &[m1(), names], &records);

    let verdicts = text.find("## Verdicts").expect("a verdict table");
    let observations = text.find("## Observations").expect("an observation table");
    let each_run = text.find("## Each run by row").expect("a run table");
    assert!(verdicts < observations && observations < each_run, "{text}");
    let table: Vec<&str> = text[observations..each_run]
        .lines()
        .filter(|line| line.starts_with("| Names |"))
        .collect();
    assert_eq!(
        table,
        [
            "| Names | real | cafe NFC / NFD: one name |",
            "| Names | real | a \\| b |",
            "| Names | absent | not run: no record |",
        ],
        "{text}"
    );
    assert!(
        !text[observations..each_run].contains("| M1 |"),
        "a judged row is not an observation: {text}"
    );
}

#[test]
fn an_observation_row_reads_recorded_only_with_its_observation_and_a_missing_verdict_reads_not_run()
{
    let names = row("Names", vec![run("real", Expect::Observe)]);
    let records = [
        record("real", &[("M1", Verdict::Pass)]),
        record(
            "simplified",
            &[("M2", Verdict::NotRun("no console".to_string()))],
        ),
    ];

    assert!(
        matches!(row_verdict(&names, &records), Verdict::NotRun(reason) if reason.contains("real") && reason.contains("Names"))
    );
    let text = render("", &[m1(), names.clone()], &records);
    assert!(
        text.lines()
            .any(|line| line.starts_with("| Names | the claim of Names |")
                && line.contains("| **not recorded**: ")),
        "{text}"
    );
    assert!(
        !text.contains("**recorded**"),
        "a row with no observation reads recorded: {text}"
    );
    assert!(
        text.lines()
            .any(|line| line == "| Names | real | what real is | recorded | not recorded |"),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line == "| Names | real | not recorded |"),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("| M1 | simplified |")
                && line.ends_with("| not run: no verdict |")),
        "{text}"
    );
    assert!(text.contains("- M2: not run: no console"), "{text}");

    let mut empty = records.clone();
    empty[0].observations.insert("Names".to_string(), vec![]);
    assert!(
        matches!(row_verdict(&names, &empty), Verdict::NotRun(reason) if reason.contains("real") && reason.contains("Names"))
    );
    let text = render("", &[m1(), names.clone()], &empty);
    assert!(
        text.lines()
            .any(|line| line.starts_with("| Names | the claim of Names |")
                && line.contains("| **not recorded**: ")),
        "{text}"
    );
    assert!(
        !text.contains("**recorded**"),
        "a row with an empty observation reads recorded: {text}"
    );
    assert!(
        text.lines()
            .any(|line| line == "| Names | real | what real is | recorded | not recorded |"),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line == "| Names | real | not recorded |"),
        "{text}"
    );

    let mut observed = records.clone();
    observed[0]
        .observations
        .insert("Names".to_string(), vec!["cafe: one name".to_string()]);
    assert_eq!(row_verdict(&names, &observed), Verdict::Pass);
    let text = render("", &[m1(), names], &observed);
    assert!(
        text.lines()
            .any(|line| line.starts_with("| Names | the claim of Names |")
                && line.ends_with("| **recorded** |")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line == "| Names | real | what real is | recorded | recorded |"),
        "{text}"
    );
}

#[test]
fn a_measured_count_is_written_beside_its_rows_verdict() {
    let r1 = row("R1", vec![run("eighty", Expect::Pass)]);
    let mut eighty = record("eighty", &[("R1", Verdict::Pass)]);
    eighty
        .counts
        .insert("R1".to_string(), "80 of 80 trials held".to_string());

    let text = render("", &[r1], &[eighty]);

    assert!(
        text.lines()
            .any(|line| line.starts_with("| R1 | the claim of R1 |")
                && line.ends_with("| **pass** (80 of 80 trials held) |")),
        "{text}"
    );
    assert!(
        text.lines().any(|line| line.starts_with("| R1 | eighty |")
            && line.ends_with("| pass (80 of 80 trials held) |")),
        "{text}"
    );
    assert!(text.contains("- R1: pass (80 of 80 trials held)"), "{text}");
}

fn section_of<'a>(text: &'a str, run: &str) -> &'a str {
    let start = text
        .find(&format!("### {run}\n"))
        .unwrap_or_else(|| panic!("a section for {run}: {text}"));
    let end = text[start + 1..]
        .find("\n### ")
        .map_or(text.len(), |end| start + 1 + end);
    &text[start..end]
}

#[test]
fn a_verdict_no_row_reads_is_marked_in_its_runs_section_with_the_runs_its_row_reads() {
    let m2 = row(
        "M2",
        vec![
            run("real", Expect::Observe),
            run("plain", Expect::Pass),
            run("ro", Expect::Fail),
        ],
    );
    let mut real = record(
        "real",
        &[
            ("M1", Verdict::Pass),
            ("M2", Verdict::Fail("EROFS, not EACCES".to_string())),
            ("M9", Verdict::Pass),
            ("M6", Verdict::Pass),
            ("M7", Verdict::Fail("e".to_string())),
        ],
    );
    real.counts.insert("M9".to_string(), "3 of 3".to_string());
    let m6 = row("M6", vec![run("one", Expect::Pass)]);
    let m7 = row(
        "M7",
        vec![
            run("a", Expect::Pass),
            run("b", Expect::Fail),
            run("c", Expect::Fail),
        ],
    );
    let records = [
        real,
        record("simplified", &[("M1", Verdict::Fail("c".to_string()))]),
        record("ro", &[("M2", Verdict::Fail("d".to_string()))]),
    ];

    let text = render("", &[m1(), m2, m6, m7], &records);

    let verdicts = |run: &str| -> Vec<String> {
        section_of(&text, run)
            .lines()
            .filter(|line| line.starts_with("- M"))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        verdicts("real"),
        [
            "- M1: pass",
            "- M2: fail: EROFS, not EACCES (no row reads this verdict; M2 reads `plain` and `ro`)",
            "- M6: pass (no row reads this verdict; M6 reads `one`)",
            "- M7: fail: e (no row reads this verdict; M7 reads `a`, `b` and `c`)",
            "- M9: pass (3 of 3) (no row reads this verdict)",
        ],
        "{text}"
    );
    assert_eq!(verdicts("simplified"), ["- M1: fail: c"], "{text}");
    assert_eq!(verdicts("ro"), ["- M2: fail: d"], "{text}");
    assert!(
        !text[..text.find("## Runs").expect("a runs heading")].contains("no row reads"),
        "only a run's section marks a verdict: {text}"
    );
}

#[test]
fn the_records_must_name_one_test_commit_with_no_uncommitted_changes() {
    let at = |name: &str, commit: &str| {
        let mut record = record(name, &[]);
        record
            .digests
            .insert("test commit".to_string(), commit.to_string());
        record
    };

    assert_eq!(
        one_commit(&[at("a", "abc"), at("b", "abc")]),
        Ok("abc".to_string())
    );
    assert!(
        matches!(one_commit(&[at("a", "abc"), at("b", "def")]), Err(why) if why.contains("abc") && why.contains("def"))
    );
    assert!(
        matches!(one_commit(&[at("a", "abc with uncommitted changes")]), Err(why) if why.contains("uncommitted"))
    );
    assert!(
        matches!(one_commit(&[at("a", "abc"), record("bare", &[])]), Err(why) if why.contains("bare"))
    );
    assert!(one_commit(&[]).is_err());
}
