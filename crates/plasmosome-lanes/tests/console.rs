mod common;

use plasmosome_lanes::console::{ConsoleRun, ProbeResult, parse_console, result_line};
use serde_json::json;

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn result(probe: &str, uid: u32) -> String {
    format!("LANE-RESULT {}", common::result_json(probe, uid))
}

fn not_run(text: &str, probes: &[&str], reason_names: &str) {
    match parse_console(text, "run-1", &ids(probes)) {
        ConsoleRun::Finished(results) => {
            panic!("expected not run ({reason_names}), it finished with {results:?}")
        }
        ConsoleRun::NotRun(reason) => assert!(
            reason.contains(reason_names),
            "the reason should mention `{reason_names}`: {reason}"
        ),
    }
}

#[test]
fn a_complete_run_after_kernel_noise_and_with_carriage_returns_finishes() {
    let text = format!(
        "[    0.000000] Booting Linux on physical CPU 0x0\r\n\
         LANE-BEGIN run-1\r\n{}\r\n{}\r\nLANE-END run-1 2\r\n\
         [    1.200000] reboot: Power down\r\n",
        result("uname", 1000),
        result("root-id", 0)
    );

    match parse_console(&text, "run-1", &ids(&["uname", "root-id"])) {
        ConsoleRun::Finished(results) => {
            assert_eq!(results.len(), 2);
            assert_eq!(results[0].probe, "uname");
            assert_eq!(results[0].uid, 1000);
            assert_eq!(results[1].probe, "root-id");
            assert_eq!(results[1].uid, 0);
            assert_eq!(results[1].result, Some(json!({"probe": "id", "ok": true})));
        }
        ConsoleRun::NotRun(reason) => panic!("a complete run did not finish: {reason}"),
    }
}

#[test]
fn a_run_with_no_probes_finishes_empty() {
    assert_eq!(
        parse_console("LANE-BEGIN run-1\nLANE-END run-1 0\n", "run-1", &[]),
        ConsoleRun::Finished(Vec::new())
    );
}

#[test]
fn a_framed_result_line_parses_back_to_the_same_result() {
    let framed = ProbeResult {
        probe: "id".to_string(),
        uid: 1000,
        exit: Some(0),
        result: Some(json!({"probe": "id", "ok": true, "uid": 1000})),
    };
    let text = format!(
        "LANE-BEGIN run-1\n{}\nLANE-END run-1 1\n",
        result_line(&framed)
    );

    assert_eq!(
        parse_console(&text, "run-1", &ids(&["id"])),
        ConsoleRun::Finished(vec![framed])
    );
}

#[test]
fn a_run_with_no_end_marker_is_not_run() {
    let text = format!("LANE-BEGIN run-1\n{}\n", result("uname", 1000));

    not_run(&text, &["uname"], "LANE-END");
}

#[test]
fn an_empty_transcript_is_not_run() {
    not_run("", &["uname"], "LANE-BEGIN");
}

#[test]
fn a_transcript_with_no_begin_marker_is_not_run() {
    let text = format!("{}\nLANE-END run-1 1\n", result("uname", 1000));

    not_run(&text, &["uname"], "LANE-BEGIN");
}

#[test]
fn a_count_that_differs_from_the_lines_seen_is_not_run() {
    let text = format!(
        "LANE-BEGIN run-1\n{}\nLANE-END run-1 2\n",
        result("uname", 1000)
    );

    not_run(&text, &["uname"], "count");
}

#[test]
fn a_begin_marker_with_another_run_id_is_not_run() {
    let text = format!(
        "LANE-BEGIN run-0\n{}\nLANE-END run-1 1\n",
        result("uname", 1000)
    );

    not_run(&text, &["uname"], "run-0");
}

#[test]
fn an_end_marker_with_another_run_id_is_not_run() {
    let text = format!(
        "LANE-BEGIN run-1\n{}\nLANE-END run-0 1\n",
        result("uname", 1000)
    );

    not_run(&text, &["uname"], "run-0");
}

#[test]
fn a_line_between_the_markers_that_is_not_a_result_is_not_run() {
    let text = format!(
        "LANE-BEGIN run-1\n[    0.9] virtiofs: mounted\n{}\nLANE-END run-1 1\n",
        result("uname", 1000)
    );

    not_run(&text, &["uname"], "virtiofs: mounted");
}

#[test]
fn a_result_line_whose_json_does_not_parse_is_not_run() {
    let text = "LANE-BEGIN run-1\nLANE-RESULT {\"probe\": \nLANE-END run-1 1\n";

    not_run(text, &["uname"], "LANE-RESULT");
}

#[test]
fn a_configured_probe_with_no_result_line_is_not_run() {
    let text = format!(
        "LANE-BEGIN run-1\n{}\nLANE-END run-1 1\n",
        result("uname", 1000)
    );

    not_run(&text, &["uname", "id"], "id");
}

#[test]
fn a_probe_whose_result_is_null_is_not_run() {
    let line = json!({"probe": "uname", "uid": 1000, "exit": 1, "result": null});
    let text = format!("LANE-BEGIN run-1\nLANE-RESULT {line}\nLANE-END run-1 1\n");

    not_run(&text, &["uname"], "uname");
}

#[test]
fn results_out_of_the_configured_order_are_not_run() {
    let text = format!(
        "LANE-BEGIN run-1\n{}\n{}\nLANE-END run-1 2\n",
        result("id", 1000),
        result("uname", 1000)
    );

    not_run(&text, &["uname", "id"], "uname");
}

#[test]
fn a_result_for_an_unconfigured_probe_is_not_run() {
    let text = format!(
        "LANE-BEGIN run-1\n{}\n{}\nLANE-END run-1 2\n",
        result("uname", 1000),
        result("extra", 1000)
    );

    not_run(&text, &["uname"], "extra");
}

#[test]
fn a_second_begin_marker_is_not_run() {
    let text = format!(
        "LANE-BEGIN run-1\nLANE-BEGIN run-1\n{}\nLANE-END run-1 1\n",
        result("uname", 1000)
    );

    not_run(&text, &["uname"], "LANE-BEGIN");
}
