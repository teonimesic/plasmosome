use serde_json::json;

use super::{finding, ids, lane_run, manifest, object, observation, runs, set, verdict};
use crate::differential::{
    Channel, Finding, Lane, ManifestRefusal, Reason, Run, RunRefusal, Subject, Verdict, judge,
};

fn not_run(probe: &str, lane: Lane) -> Finding {
    finding(probe, Reason::NotRun { lane })
}

fn noise(probe: &str, lane: Lane, field: &str) -> Finding {
    finding(
        probe,
        Reason::Noise {
            lane,
            field: field.to_string(),
        },
    )
}

fn never_discriminates(runs: &mut [Run], probe: &str) {
    set(
        runs,
        Lane::Open,
        probe,
        "outcome",
        json!("name does not resolve"),
    );
}

fn per_run(runs: &mut [Run], lanes: &[Lane], probe: &str, field: &str) {
    for (index, run) in runs.iter_mut().enumerate() {
        if lanes.contains(&run.lane) {
            let observed = run.observations.iter_mut().find(|seen| seen.probe == probe);
            let observed = observed.expect("the run observed that probe");
            observed
                .fields
                .insert(field.to_string(), json!(40_000 + index));
        }
    }
}

#[test]
fn item_12_staged_suite_is_not_run_and_still_lists_its_failures() {
    let mut runs = runs();
    let crashed = observation(&mut runs, Lane::Absent, 1, "refusal-host-1");
    crashed.completed = false;
    crashed.fields.clear();
    for lane in Lane::ALL {
        for run in 0..2 {
            let failed = observation(&mut runs, lane, run, "refusal-host-2");
            failed.failed_before_step = true;
            failed.fields = object(json!({"errno": "EACCES"}));
        }
    }
    never_discriminates(&mut runs, "refusal-host-3");
    never_discriminates(&mut runs, "facility-1");
    per_run(&mut runs, &[Lane::Open], "facility-1", "random");
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::NotRun {
            not_run: vec![
                not_run("refusal-host-1", Lane::Absent),
                not_run("refusal-host-2", Lane::Cell),
                not_run("refusal-host-2", Lane::Open),
                not_run("refusal-host-2", Lane::Absent),
            ],
            failures: vec![
                finding("refusal-host-3", Reason::NoDiscrimination),
                noise("facility-1", Lane::Open, "random"),
            ],
        }
    );
}

#[test]
fn a_probe_missing_from_every_run_is_not_run_in_every_lane() {
    let mut runs = runs();
    for run in &mut runs {
        run.observations.retain(|seen| seen.probe != "op-times");
    }
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::NotRun {
            not_run: Lane::ALL.map(|lane| not_run("op-times", lane)).to_vec(),
            failures: vec![],
        }
    );
}

#[test]
fn a_reach_probe_equal_in_the_open_and_absent_lanes_fails() {
    let mut runs = runs();
    never_discriminates(&mut runs, "disturbance-path-3");
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![finding(
            "disturbance-path-3",
            Reason::NoDiscrimination
        )])
    );
}

#[test]
fn a_probe_whose_two_runs_differ_is_noise_in_that_lane() {
    let mut runs = runs();
    observation(&mut runs, Lane::Open, 1, "hint-cgroup")
        .fields
        .insert("controllers".to_string(), json!("cpu memory"));
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![noise("hint-cgroup", Lane::Open, "controllers")])
    );
}

#[test]
fn a_rule_in_the_manifest_loosens_no_comparison() {
    let mut runs = runs();
    set(&mut runs, Lane::Cell, "hint-self", "pid", json!(201));
    let times = &mut observation(&mut runs, Lane::Open, 1, "op-times").fields;
    times.insert("times".to_string(), json!([10_001, 20_001]));
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![
            super::differs("hint-self", "pid"),
            noise("op-times", Lane::Open, "times"),
        ])
    );
}

#[test]
fn a_channel_one_probe_short_is_refused_before_any_run_is_judged() {
    let mut manifest = manifest();
    manifest.probes.retain(|probe| probe.id != "op-times");
    let mut runs = runs();
    for run in &mut runs {
        run.observations.retain(|seen| seen.probe != "op-times");
    }
    assert_eq!(
        judge(&manifest, &runs),
        Err(RunRefusal::Manifest(ManifestRefusal::ChannelShort {
            channel: Channel::Operation,
            count: 2
        }))
    );
}

#[test]
fn a_lane_with_no_runs_leaves_every_probe_and_the_transcript_not_run() {
    let mut runs = runs();
    runs.retain(|run| run.lane != Lane::Cell);
    let mut expected: Vec<Finding> = ids().map(|id| not_run(id, Lane::Cell)).collect();
    expected.push(Finding {
        subject: Subject::Transcript,
        reason: Reason::NotRun { lane: Lane::Cell },
    });
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::NotRun {
            not_run: expected,
            failures: vec![],
        }
    );
}

#[test]
fn the_transcript_is_compared_exactly_and_is_not_run_without_one() {
    let transcript = |reason| Finding {
        subject: Subject::Transcript,
        reason,
    };
    let field = || "exit_statuses".to_string();
    for missing in [2, 3] {
        let mut runs = runs();
        let cell = runs[1].transcript.as_mut().expect("a transcript");
        cell.insert(field(), json!([1]));
        runs[missing].transcript = None;
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::NotRun {
                not_run: vec![transcript(Reason::NotRun { lane: Lane::Open })],
                failures: vec![
                    transcript(Reason::Noise {
                        lane: Lane::Cell,
                        field: field()
                    }),
                    transcript(Reason::Differs { field: field() }),
                ],
            },
            "open run {missing} has no transcript"
        );
    }
}

#[test]
fn two_runs_from_one_boot_are_refused_in_one_lane_or_across_two() {
    for (from, to, first, second) in [
        (2, 3, Lane::Open, Lane::Open),
        (1, 4, Lane::Cell, Lane::Absent),
    ] {
        let mut runs = runs();
        runs[to].boot_id = runs[from].boot_id.clone();
        assert_eq!(
            judge(&manifest(), &runs),
            Err(RunRefusal::SameBoot {
                boot_id: runs[from].boot_id.clone(),
                first,
                second
            })
        );
    }
}

#[test]
fn a_lane_with_one_run_or_three_is_refused() {
    let mut one = runs();
    one.remove(2);
    let mut three = runs();
    three.push(lane_run(Lane::Absent, 2));
    for (runs, lane, count) in [(one, Lane::Open, 1), (three, Lane::Absent, 3)] {
        assert_eq!(
            judge(&manifest(), &runs),
            Err(RunRefusal::RunCount { lane, count })
        );
    }
}

#[test]
fn an_undeclared_or_repeated_observation_is_refused() {
    let mut undeclared = runs();
    let mut ghost = undeclared[0].observations[0].clone();
    ghost.probe = "ghost".to_string();
    undeclared[0].observations.push(ghost);
    let mut repeated = runs();
    let again = repeated[4].observations[2].clone();
    repeated[4].observations.push(again);
    let probe = |id: &str| id.to_string();
    for (runs, refusal) in [
        (
            undeclared,
            RunRefusal::UnknownProbe {
                lane: Lane::Cell,
                probe: probe("ghost"),
            },
        ),
        (
            repeated,
            RunRefusal::DuplicateObservation {
                lane: Lane::Absent,
                probe: probe("refusal-host-3"),
            },
        ),
    ] {
        assert_eq!(judge(&manifest(), &runs), Err(refusal));
    }
}
