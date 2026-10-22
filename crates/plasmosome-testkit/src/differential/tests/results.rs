use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::symlink;

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{lane_run, load_value, manifest, manifest_json, observation, runs, set};
use crate::differential::results::write_with;
use crate::differential::{
    Digests, Lane, LaneConfig, Manifest, Observation, Outcome, ReadRefusal, Record, ResultsRefusal,
    Rule, RuleKind, Run, Subject, judge, write_results,
};

const DIGESTS: &str = "image,kernel,libkrun,fixtures,harness,session,probe suite,manifest";
const ADMITS: &str = "and no allowance admits it";
const EXISTS: ErrorKind = ErrorKind::AlreadyExists;

fn listed(dir: &std::path::Path) -> usize {
    fs::read_dir(dir).expect("a listing").count()
}

fn digest(number: usize) -> String {
    format!("{number:064x}")
}

fn digest_mut<'a>(digests: &'a mut Digests, name: &str) -> &'a mut String {
    match name {
        "image" => &mut digests.image,
        "kernel" => &mut digests.kernel,
        "libkrun" => &mut digests.libkrun,
        "fixtures" => &mut digests.fixtures,
        "harness" => digests.harness.as_mut().expect("a harness digest"),
        "session" => digests.session.as_mut().expect("a session digest"),
        "probe suite" => &mut digests.probe_suite,
        _ => &mut digests.manifest,
    }
}

fn config(lane: Lane) -> LaneConfig {
    let settings = [("cmdline", format!("lane={lane:?}")), ("vcpus", "2".into())];
    let settings = settings.map(|(key, value)| (key.to_string(), value)).into();
    LaneConfig { lane, settings }
}

fn record() -> Record {
    let (image, kernel, libkrun, fixtures) = (digest(1), digest(2), digest(3), digest(4));
    let (harness, session) = (Some(digest(5)), Some(digest(6)));
    let (probe_suite, manifest) = (digest(7), digest(8));
    let digests = Digests {
        image,
        kernel,
        libkrun,
        fixtures,
        harness,
        session,
        probe_suite,
        manifest,
    };
    let configs = Lane::ALL.into_iter().map(config).collect();
    let (date, name) = ("2026-10-22".to_string(), "staged-suite".to_string());
    Record {
        date,
        name,
        configs,
        digests,
    }
}

fn read<'a>(manifest: &'a Manifest, runs: &'a [Run]) -> Outcome<'a> {
    Outcome::Read { manifest, runs }
}

fn reader(lane: Lane, subject: Subject, reason: &str) -> ReadRefusal {
    let reason = reason.to_string();
    ReadRefusal {
        lane,
        subject,
        reason,
    }
}

fn written(outcome: Outcome<'_>, record: &Record) -> Result<String, ResultsRefusal> {
    let dir = TempDir::new().expect("a temporary directory");
    let path = write_results(outcome, record, dir.path())?;
    Ok(fs::read_to_string(path).expect("the results file reads back"))
}

fn text(manifest: &Manifest, runs: &[Run]) -> String {
    written(read(manifest, runs), &record()).expect("the results are written")
}

fn refused(outcome: Outcome<'_>, record: &Record) -> String {
    format!("{:?}", written(outcome, record).expect_err("not written"))
}

fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let marker = format!("\n{heading}\n\n");
    let start = text.find(&marker).expect("the heading is in the file") + marker.len();
    let rest = &text[start..];
    rest.find("\n#").map_or(rest, |end| &rest[..=end])
}

fn block<'a>(section: &'a str, language: &str) -> &'a str {
    let body = section.split(&format!("```{language}\n")).nth(1);
    let body = body.expect("the block opens").split("\n```\n").next();
    body.expect("the block closes")
}

fn json_in(section: &str) -> Value {
    serde_json::from_str(block(section, "json")).expect("the block holds JSON")
}

fn why(text: &str) -> &str {
    block(section(text, "## Verdict"), "text")
}

fn summary(verdict: &str, probes: &str, [cell, open, absent]: [usize; 3]) -> String {
    let runs = format!("cell {cell}, open {open}, absent {absent}");
    format!("\n- Verdict: **{verdict}**\n- Probes: {probes}\n- Runs: {runs}\n")
}

fn statuses(run: &Value) -> Vec<[&str; 2]> {
    let observations = run["observations"].as_array().expect("a list");
    observations.iter().map(pair).collect()
}

fn pair(seen: &Value) -> [&str; 2] {
    ["probe", "status"].map(|key| seen[key].as_str().unwrap_or("?"))
}

fn completed(seen: &Observation) -> Value {
    json!({"probe": seen.probe, "status": "completed", "fields": seen.fields})
}

#[test]
fn a_passing_run_writes_every_field_spec_027_names_as_run() {
    let (manifest, runs) = (manifest(), runs());
    let text = text(&manifest, &runs);
    assert!(text.starts_with("# Membrane differential: staged-suite\n\nSpec 027"));
    assert!(text.contains("run on 2026-10-22."));
    assert!(text.contains(&summary("pass", "17 in the manifest", [2, 2, 2])));
    assert!(section(&text, "## Verdict").starts_with("Every probe and the transcript ran"));
    for lane in Lane::ALL {
        let heading = format!("### {lane:?}").to_lowercase();
        let settings = json!(config(lane).settings);
        assert_eq!(json_in(section(&text, &heading)), settings);
    }
    let mut digests = "| Input | SHA-256 |\n| --- | --- |\n".to_string();
    for (index, name) in DIGESTS.split(',').enumerate() {
        digests.push_str(&format!("| {name} | `{}` |\n", digest(index + 1)));
    }
    assert_eq!(section(&text, "## Digests"), digests + "\n");
    let table = section(&text, "## Normalization table as run");
    assert_eq!(json_in(table), manifest_json()["rules"]);
    for (index, run) in runs.iter().enumerate() {
        let heading = format!("### {:?}, run {}", run.lane, index % 2 + 1).to_lowercase();
        let observations: Vec<Value> = run.observations.iter().map(completed).collect();
        let transcript = json!({"status": "completed", "fields": run.transcript});
        let boot_id = &run.boot_id;
        let run =
            json!({"boot_id": boot_id, "observations": observations, "transcript": transcript});
        assert_eq!(json_in(section(&text, &heading)), run);
    }
    let cell = json_in(section(&text, "### cell, run 1"));
    let pids = &cell["observations"][12]["fields"]["pids"];
    assert_eq!(pids, &json!([1, 100, 200]));
}

#[test]
fn the_normalization_table_as_run_lists_a_rule_of_each_of_the_thirteen_kinds() {
    let mut manifest = manifest();
    let field = |kind: RuleKind| format!("by-{}", kind.name());
    let probe = "hint-env".to_string();
    let rule = |kind| Rule {
        probe: probe.clone(),
        field: field(kind),
        kind,
    };
    manifest.rules = RuleKind::ALL.into_iter().map(rule).collect();
    let text = text(&manifest, &runs());
    let table = section(&text, "## Normalization table as run");
    let row = |kind: RuleKind| json!({"probe": probe, "field": field(kind), "kind": kind.name()});
    assert_eq!(json_in(table), json!(RuleKind::ALL.map(row)));
    assert!(table.contains("\"kind\": \"peer_group\""));
}

#[test]
fn a_manifest_refused_at_load_is_a_failed_run_with_no_probes() {
    let mut short = manifest_json();
    let probes = short["probes"].as_array_mut().expect("a list");
    probes.retain(|probe| probe["id"] != "refusal-host-3");
    let refusal = load_value(&short).expect_err("a channel one probe short is refused");
    let mut record = record();
    record.configs.clear();
    (record.digests.harness, record.digests.session) = (None, None);
    let text = written(Outcome::ManifestRefused(&refusal), &record).expect("written");
    let probes = "0, since the manifest was refused before any run";
    assert!(text.contains(&summary("fail", probes, [0, 0, 0])));
    let verdict = section(&text, "## Verdict");
    assert!(verdict.starts_with("The runs were refused, which is a failed run:"));
    let channel = "ChannelShort { channel: Refusal, count: 2 }";
    assert_eq!(why(&text), format!("load refused the manifest: {channel}"));
    assert_eq!(section(&text, "### absent"), "No configuration given.\n\n");
    let none = "none: no run carries a transcript";
    let digests = format!("| harness | {none} |\n| session | {none} |\n");
    assert!(section(&text, "## Digests").contains(&digests));
    let refused = "None: the manifest was refused before any run.\n";
    let table = section(&text, "## Normalization table as run");
    assert_eq!(table, format!("{refused}\n"));
    assert_eq!(section(&text, "## Runs"), refused);
}

#[test]
fn a_manifest_with_no_probes_is_written_as_a_failed_run_and_never_a_pass() {
    let mut empty = manifest();
    (empty.probes, empty.rules, empty.allowances) = (vec![], vec![], vec![]);
    let text = text(&empty, &[]);
    assert!(text.contains(&summary("fail", "0 in the manifest", [0, 0, 0])));
    let channel = "ChannelShort { channel: Refusal, count: 0 }";
    assert_eq!(
        why(&text),
        format!("judge refused the runs: Manifest({channel})")
    );
    assert!(!text.contains("**pass**"));
}

#[test]
fn every_refusal_by_judge_is_a_failed_run_that_still_lists_every_run() {
    let mut same_boot = runs();
    same_boot[3].boot_id = same_boot[0].boot_id.clone();
    let mut three = runs();
    three.push(lane_run(Lane::Open, 2));
    let mut unknown = runs();
    let mut ghost = unknown[2].observations[0].clone();
    ghost.probe = "ghost".to_string();
    unknown[2].observations.push(ghost);
    let mut cannot_hold = runs();
    set(
        &mut cannot_hold,
        Lane::Cell,
        "hint-self",
        "pid",
        json!("one"),
    );
    let mut bad = manifest();
    bad.rules[0].probe = "ghost".to_string();
    let cases = [
        (manifest(), same_boot, "SameBoot"),
        (manifest(), three, "RunCount { lane: Open, count: 3 }"),
        (
            manifest(),
            unknown,
            "UnknownProbe { lane: Open, probe: \"ghost\" }",
        ),
        (manifest(), cannot_hold, "RuleCannotHold"),
        (bad, runs(), "Manifest(UnknownProbe"),
    ];
    for (manifest, runs, refusal) in &cases {
        let judged = judge(manifest, runs).expect_err("the runs are refused");
        assert!(format!("{judged:?}").starts_with(refusal));
        let text = text(manifest, runs);
        assert!(text.contains("\n- Verdict: **fail**\n- Probes: 17 in the manifest\n"));
        assert_eq!(why(&text), format!("judge refused the runs: {judged:?}"));
        let first = json_in(section(&text, "### cell, run 1"));
        assert_eq!(first["boot_id"], json!(runs[0].boot_id));
    }
    assert!(text(&cases[1].0, &cases[1].1).contains("\n### open, run 3\n"));
    let text = text(&cases[2].0, &cases[2].1);
    let open = json_in(section(&text, "### open, run 1"));
    assert_eq!(statuses(&open)[17], ["ghost", "undeclared"]);
}

#[test]
fn a_reader_refusal_is_a_failed_run_naming_the_lane_and_the_subject() {
    let manifest = manifest();
    let reason = "a request the recorded session does not hold";
    let probe = Subject::Probe("op-times".to_string());
    let cases = [
        (probe, "probe \"op-times\""),
        (Subject::Transcript, "the transcript"),
    ];
    for (subject, named) in cases {
        let refusal = reader(Lane::Open, subject, reason);
        let refusal = &refusal;
        let outcome = Outcome::ReadRefused {
            manifest: &manifest,
            refusal,
        };
        let mut record = record();
        record.configs = vec![config(Lane::Open)];
        let text = written(outcome, &record).expect("written");
        let unread = "- Runs: not read, since a reader refused a lane's output\n";
        assert!(text.contains(&format!("- Probes: 17 in the manifest\n{unread}")));
        assert!(text.contains("\n- Verdict: **fail**\n"));
        let expected = format!("a reader refused the open lane's output for {named}: {reason:?}");
        assert_eq!(why(&text), expected);
        let table = section(&text, "## Normalization table as run");
        assert_eq!(json_in(table), manifest_json()["rules"]);
        let runs = "None: a reader refused a lane's output, so no run was read.\n";
        assert_eq!(section(&text, "## Runs"), runs);
        record.configs.clear();
        assert_eq!(refused(outcome, &record), "MissingConfig { lane: Open }");
    }
    let refusal = &reader(Lane::Cell, Subject::Probe("op-times".to_string()), "");
    let mut record = record();
    (record.digests.harness, record.digests.session) = (None, None);
    let outcome = Outcome::ReadRefused {
        manifest: &manifest,
        refusal,
    };
    let text = written(outcome, &record).expect("written");
    let digests = "| harness | none given |\n| session | none given |\n";
    assert!(section(&text, "## Digests").contains(digests));
    let refusal = &reader(Lane::Cell, Subject::Transcript, "");
    let mut record = self::record();
    record.digests.session = None;
    let outcome = Outcome::ReadRefused {
        manifest: &manifest,
        refusal,
    };
    assert_eq!(
        refused(outcome, &record),
        "MissingDigest { name: \"session\" }"
    );
}

#[test]
fn a_failed_verdict_writes_each_finding_in_words() {
    let mut runs = runs();
    let open = json!("name does not resolve");
    set(&mut runs, Lane::Open, "refusal-host-3", "outcome", open);
    let opt = json!({"source": "plasmosome", "fstype": "virtiofs", "options": "ro"});
    set(&mut runs, Lane::Cell, "hint-mounts", "/opt", opt);
    let workspace =
        json!({"source": "workspace", "fstype": "fuse", "options": "rw", "subtype": "x"});
    set(
        &mut runs,
        Lane::Absent,
        "hint-mounts",
        "/workspace",
        workspace,
    );
    let cgroup = json!("/other");
    set(&mut runs, Lane::Absent, "hint-cgroup", "path", cgroup);
    let noisy = observation(&mut runs, Lane::Open, 1, "hint-env");
    noisy.fields.insert("HOME".to_string(), json!("/root"));
    for run in runs.iter_mut().filter(|run| run.lane == Lane::Cell) {
        let transcript = run.transcript.as_mut().expect("a transcript");
        transcript.insert("exit_statuses".to_string(), json!([1]));
    }
    let text = text(&manifest(), &runs);
    assert!(text.contains("\n- Verdict: **fail**\n"));
    assert!(section(&text, "## Verdict").starts_with("Failures:\n\n"));
    let differs = |field: &str| {
        format!("the cell lane differs from the absent lane in field \"{field}\", {ADMITS}")
    };
    let source = "the cell lane does not show the neutral source of the mount at \"/opt\"";
    let subtype =
        "the absent lane does not show the neutral subtype of the mount at \"/workspace\"";
    let lines = [
        "probe \"refusal-host-3\": no discrimination: an open run and an absent run are equal",
        &format!("probe \"hint-mounts\": {source}"),
        &format!("probe \"hint-mounts\": {subtype}"),
        &format!("probe \"hint-mounts\": {}", differs("/opt")),
        &format!("probe \"hint-mounts\": {}", differs("/workspace")),
        "probe \"hint-cgroup\": the absent lane does not show the neutral cgroup path",
        &format!("probe \"hint-cgroup\": {}", differs("path")),
        "probe \"hint-env\": noise: the open lane's two runs differ in field \"HOME\"",
        &format!("the transcript: {}", differs("exit_statuses")),
    ];
    assert_eq!(why(&text), lines.join("\n"));
}

#[test]
fn a_not_run_verdict_lists_what_did_not_run_and_the_failures() {
    let mut runs = runs();
    observation(&mut runs, Lane::Absent, 1, "refusal-host-1").completed = false;
    let noisy = observation(&mut runs, Lane::Open, 1, "hint-env");
    noisy.fields.insert("HOME".to_string(), json!("/root"));
    let text = text(&manifest(), &runs);
    assert!(text.contains("\n- Verdict: **not run**\n"));
    let not_run = "probe \"refusal-host-1\": not run in the absent lane";
    let noise = "probe \"hint-env\": noise: the open lane's two runs differ in field \"HOME\"";
    let failures = format!("Failures:\n\n```text\n{noise}\n```\n\n");
    let expected = format!("Not run:\n\n```text\n{not_run}\n```\n\n{failures}");
    assert_eq!(section(&text, "## Verdict"), expected);
}

#[test]
fn a_lane_with_no_runs_reads_no_runs_and_needs_no_configuration() {
    let runs: Vec<Run> = runs()
        .into_iter()
        .filter(|run| run.lane != Lane::Cell)
        .collect();
    let mut record = record();
    record.configs.retain(|config| config.lane != Lane::Cell);
    let text = written(read(&manifest(), &runs), &record).expect("written");
    assert!(text.contains(&summary("not run", "17 in the manifest", [0, 2, 2])));
    let not_run: Vec<&str> = why(&text).lines().collect();
    assert_eq!(not_run.len(), 18);
    assert_eq!(
        not_run[0],
        "probe \"refusal-host-1\": not run in the cell lane"
    );
    assert_eq!(not_run[17], "the transcript: not run in the cell lane");
    assert!(section(&text, "## Verdict").ends_with("Failures: none.\n\n"));
    assert!(text.contains("## Lanes\n\n### cell\n\nNo configuration given.\n\n### open\n"));
    assert!(text.contains("## Runs\n\n### cell\n\nNo runs.\n\n### open, run 1\n"));
    record
        .configs
        .extend([config(Lane::Cell), config(Lane::Cell)]);
    let twice = refused(read(&manifest(), &runs), &record);
    assert_eq!(twice, "DuplicateConfig { lane: Cell }");
}

#[test]
fn every_status_of_a_probe_and_of_the_transcript_is_written() {
    let mut runs = runs();
    observation(&mut runs, Lane::Cell, 0, "refusal-host-1").completed = false;
    observation(&mut runs, Lane::Cell, 0, "refusal-host-2").failed_before_step = true;
    let both = observation(&mut runs, Lane::Cell, 0, "refusal-host-3");
    (both.completed, both.failed_before_step) = (false, true);
    for run in &mut runs {
        run.observations.retain(|seen| seen.probe != "facility-1");
    }
    runs[0].transcript = None;
    let text = text(&manifest(), &runs);
    assert!(text.contains(&summary("not run", "17 in the manifest", [2, 2, 2])));
    let run = json_in(section(&text, "### cell, run 1"));
    for (index, seen) in runs[0].observations[..3].iter().enumerate() {
        assert_eq!(run["observations"][index]["fields"], json!(seen.fields));
    }
    let both = "not run: no completion marker, and failed before its step";
    let expected = [
        ["refusal-host-1", "not run: no completion marker"],
        ["refusal-host-2", "not run: failed before its step"],
        ["refusal-host-3", both],
        ["facility-2", "completed"],
    ];
    assert_eq!(statuses(&run)[..4], expected);
    let unseen = json!({"probe": "facility-1", "status": "not run: not observed"});
    assert_eq!(run["observations"][16], unseen);
    assert_eq!(
        run["transcript"],
        json!({"status": "not run: no transcript"})
    );
}

#[test]
fn the_rerun_list_must_name_every_lane_that_ran_and_every_digest() {
    let (manifest, runs) = (manifest(), runs());
    let outcome = read(&manifest, &runs);
    for lane in Lane::ALL {
        let mut record = record();
        record.configs.retain(|config| config.lane != lane);
        let missing = format!("MissingConfig {{ lane: {lane:?} }}");
        assert_eq!(refused(outcome, &record), missing);
        record.configs.extend([config(lane), config(lane)]);
        let twice = format!("DuplicateConfig {{ lane: {lane:?} }}");
        assert_eq!(refused(outcome, &record), twice);
    }
    let hex = "f".repeat(63);
    let (upper, letter) = ("F".repeat(64), "g".repeat(64));
    let bad = [
        "",
        &hex,
        &format!("{hex}ff"),
        &upper,
        &letter,
        &format!("{hex}/"),
    ];
    for name in DIGESTS.split(',') {
        for value in bad {
            let mut record = record();
            *digest_mut(&mut record.digests, name) = value.to_string();
            let named = format!("BadDigest {{ name: {name:?} }}");
            assert_eq!(refused(outcome, &record), named);
        }
    }
    let mut record = record();
    record.configs[0].settings.clear();
    assert_eq!(refused(outcome, &record), "EmptyConfig { lane: Cell }");
    let mut record = self::record();
    record.digests.harness = None;
    let mut some = runs.clone();
    some[0].transcript = None;
    assert_eq!(
        refused(read(&manifest, &some), &record),
        "MissingDigest { name: \"harness\" }"
    );
    let mut untranscribed = runs.clone();
    untranscribed
        .iter_mut()
        .for_each(|run| run.transcript = None);
    record.digests.session = None;
    let text = written(read(&manifest, &untranscribed), &record).expect("no transcript");
    assert!(text.contains("| harness | none: no run carries a transcript |\n"));
    assert!(text.contains(&summary("not run", "17 in the manifest", [2, 2, 2])));
    let lanes = ["cell", "open", "absent"];
    let not_run = lanes.map(|lane| format!("the transcript: not run in the {lane} lane"));
    assert_eq!(why(&text), not_run.join("\n"));
}

#[test]
fn the_file_is_named_by_date_and_name_and_never_overwritten() {
    let (manifest, runs) = (manifest(), runs());
    let outcome = read(&manifest, &runs);
    let dir = TempDir::new().expect("a temporary directory");
    let dates = ["2026-10-22", "2024-02-29", "2000-02-29", "2026-12-31"];
    for (index, date) in dates.into_iter().enumerate() {
        let (mut record, name) = (record(), format!("suite-{index}"));
        (record.date, record.name) = (date.to_string(), name.clone());
        let path = write_results(outcome, &record, dir.path()).expect("written");
        assert_eq!(path, dir.path().join(format!("{date}-{name}.md")));
        let first = fs::read(&path).expect("the file reads back");
        let again = write_results(outcome, &record, dir.path()).expect_err("never overwritten");
        assert!(matches!(again, ResultsRefusal::Io(error) if error.kind() == EXISTS));
        assert_eq!(fs::read(&path).expect("the file reads back"), first);
        assert_eq!(listed(dir.path()), index + 1);
    }
    let empty = TempDir::new().expect("a temporary directory");
    let dates = [
        "",
        "2026-1-022",
        "20261022xx",
        "2026/10/22",
        "2026-10-2a",
        "+026-10-22",
        "2026-10-22-",
        "２０２６-10-22",
        "2026-02-30",
        "2026-13-45",
        "0000-00-00",
        "9999-99-99",
        "2026-22-10",
        "2026-02-29",
        "1900-02-29",
        "2026-04-31",
        "2026-01-00",
        "2026-00-10",
    ];
    for date in dates {
        let mut record = record();
        record.date = date.to_string();
        let refusal = write_results(outcome, &record, empty.path()).expect_err("a bad date");
        assert_eq!(
            format!("{refusal:?}"),
            format!("BadDate {{ date: {date:?} }}")
        );
    }
    for name in ["", "Suite", "a/b", "../a", "a b", "a_b", "a.md", "é"] {
        let mut record = record();
        record.name = name.to_string();
        let refusal = write_results(outcome, &record, empty.path()).expect_err("a bad name");
        assert_eq!(
            format!("{refusal:?}"),
            format!("BadName {{ name: {name:?} }}")
        );
    }
    assert_eq!(listed(empty.path()), 0);
    let missing = empty.path().join("missing");
    let refusal = write_results(outcome, &record(), &missing).expect_err("no such directory");
    assert!(matches!(refusal, ResultsRefusal::Io(error) if error.kind() == ErrorKind::NotFound));
}

#[test]
fn a_link_at_the_name_is_never_written_through() {
    let (manifest, runs) = (manifest(), runs());
    let dir = TempDir::new().expect("a temporary directory");
    let target = dir.path().join("target");
    symlink(&target, dir.path().join("2026-10-22-staged-suite.md")).expect("a dangling link");
    for kept in [None, Some("kept")] {
        if let Some(kept) = kept {
            fs::write(&target, kept).expect("the target is written");
        }
        let refusal = write_results(read(&manifest, &runs), &record(), dir.path());
        let refusal = refusal.expect_err("a link at the name");
        assert!(matches!(refusal, ResultsRefusal::Io(error) if error.kind() == EXISTS));
        assert_eq!(fs::read_to_string(&target).ok().as_deref(), kept);
        assert_eq!(listed(dir.path()), 1 + usize::from(kept.is_some()));
    }
}

#[test]
fn a_write_that_fails_partway_leaves_no_file_and_a_retry_writes_it() {
    let (manifest, runs) = (manifest(), runs());
    let dir = TempDir::new().expect("a temporary directory");
    let full = |file: &mut File, text: &[u8]| {
        file.write_all(&text[..512])?;
        Err(io::Error::other("the disk is full"))
    };
    let refusal = write_with(read(&manifest, &runs), &record(), dir.path(), full);
    let refusal = format!("{:?}", refusal.expect_err("the write fails"));
    assert!(refusal.starts_with("Io(") && refusal.contains("the disk is full"));
    assert_eq!(listed(dir.path()), 0);
    let path = write_results(read(&manifest, &runs), &record(), dir.path()).expect("a retry");
    let whole = fs::read_to_string(path).expect("the file reads back");
    assert_eq!(whole, text(&manifest, &runs));
    assert_eq!(listed(dir.path()), 1);
}

fn one_verdict_and_closed_blocks(text: &str) {
    let verdicts: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("- V"))
        .collect();
    assert_eq!(verdicts, ["- Verdict: **fail**"]);
    let fences: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("```"))
        .collect();
    let closed = fences.iter().filter(|line| **line == "```").count();
    assert_eq!(2 * closed, fences.len());
}

#[test]
fn names_and_values_cannot_break_the_file_apart() {
    let mut runs = runs();
    let fence = "\n```\n## Runs\n# x | y `z`";
    set(&mut runs, Lane::Open, "hint-env", "HOME", json!(fence));
    runs[2].boot_id = format!("open{fence}");
    let pass = "x\n```\n\n- Verdict: **pass**";
    set(&mut runs, Lane::Cell, "hint-env", pass, json!(1));
    let mut record = record();
    let settings = &mut record.configs[1].settings;
    settings.insert("cmdline".to_string(), fence.to_string());
    let manifest = manifest();
    let text = written(read(&manifest, &runs), &record).expect("written");
    one_verdict_and_closed_blocks(&text);
    let differs = format!("the cell lane differs from the absent lane in field {pass:?}");
    assert_eq!(
        why(&text),
        format!("probe \"hint-env\": {differs}, {ADMITS}")
    );
    let refusal = reader(Lane::Open, Subject::Transcript, pass);
    let refusal = &refusal;
    let outcome = Outcome::ReadRefused {
        manifest: &manifest,
        refusal,
    };
    let unread = written(outcome, &record).expect("written");
    one_verdict_and_closed_blocks(&unread);
    let output = "a reader refused the open lane's output for the transcript";
    assert_eq!(why(&unread), format!("{output}: {pass:?}"));
    let headings: Vec<&str> = text.lines().filter(|line| line.starts_with('#')).collect();
    let mut expected = vec!["# Membrane differential: staged-suite".to_string()];
    let lanes = ["cell", "open", "absent"];
    expected.extend(["Verdict", "Lanes"].map(|heading| format!("## {heading}")));
    expected.extend(lanes.map(|lane| format!("### {lane}")));
    let later = ["Digests", "Normalization table as run", "Runs"];
    expected.extend(later.map(|heading| format!("## {heading}")));
    for lane in lanes {
        expected.extend([1, 2].map(|run| format!("### {lane}, run {run}")));
    }
    assert_eq!(headings, expected);
    let open = json_in(section(&text, "### open, run 1"));
    assert_eq!(open["boot_id"], json!(runs[2].boot_id));
    assert_eq!(open["observations"][11]["fields"]["HOME"], json!(fence));
    assert_eq!(json_in(section(&text, "### open"))["cmdline"], json!(fence));
}
