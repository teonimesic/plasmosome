use serde_json::json;

use super::{finding, ids, lane_run, manifest, object, observation, runs, set, verdict};
use crate::differential::{
    Channel, Finding, Lane, Manifest, ManifestRefusal, MountKey, NeutralName, Reason, Rule,
    RuleKind, Run, RunRefusal, Subject, Verdict, judge,
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

fn pid_rule(probe: &str, field: &str) -> Manifest {
    let mut manifest = manifest();
    manifest.rules.push(Rule {
        probe: probe.to_string(),
        field: field.to_string(),
        kind: RuleKind::Pid,
    });
    manifest
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
fn a_reach_probe_that_differs_only_in_a_ruled_field_does_not_discriminate() {
    let manifest = pid_rule("refusal-host-1", "pid");
    let mut runs = runs();
    never_discriminates(&mut runs, "refusal-host-1");
    for (lane, pid) in [(Lane::Cell, 300), (Lane::Open, 310), (Lane::Absent, 300)] {
        set(&mut runs, lane, "refusal-host-1", "pid", json!(pid));
    }
    assert_eq!(
        verdict(&manifest, &runs),
        Verdict::Fail(vec![finding("refusal-host-1", Reason::NoDiscrimination)])
    );
}

#[test]
fn a_reach_probe_that_reports_only_ruled_fields_does_not_discriminate() {
    let manifest = pid_rule("refusal-host-1", "pid");
    let mut runs = runs();
    for (lane, pid) in [(Lane::Cell, 300), (Lane::Open, 310), (Lane::Absent, 300)] {
        for run in 0..2 {
            let fields = &mut observation(&mut runs, lane, run, "refusal-host-1").fields;
            *fields = object(json!({"pid": pid}));
        }
    }
    assert_eq!(
        verdict(&manifest, &runs),
        Verdict::Fail(vec![finding("refusal-host-1", Reason::NoDiscrimination)])
    );
}

#[test]
fn a_field_another_probes_rule_covers_still_discriminates() {
    let manifest = manifest();
    assert!(manifest.rules.iter().any(|rule| rule.field == "pid"));
    let mut runs = runs();
    never_discriminates(&mut runs, "refusal-host-1");
    for (lane, pid) in [(Lane::Cell, 300), (Lane::Open, 310), (Lane::Absent, 300)] {
        set(&mut runs, lane, "refusal-host-1", "pid", json!(pid));
    }
    assert_eq!(verdict(&manifest, &runs), Verdict::Pass);
}

#[test]
fn a_ruled_field_of_a_reach_probe_is_still_noise_and_still_a_difference() {
    let manifest = pid_rule("refusal-host-1", "pids");
    let staged = |cell: [u32; 2], second_open: [u32; 2]| {
        let mut runs = runs();
        for (lane, pids) in [
            (Lane::Cell, cell),
            (Lane::Open, [310, 311]),
            (Lane::Absent, [300, 301]),
        ] {
            set(&mut runs, lane, "refusal-host-1", "pids", json!(pids));
        }
        let open = &mut observation(&mut runs, Lane::Open, 1, "refusal-host-1").fields;
        open.insert("pids".to_string(), json!(second_open));
        runs
    };
    assert_eq!(
        verdict(&manifest, &staged([300, 300], [310, 311])),
        Verdict::Fail(vec![super::differs("refusal-host-1", "pids")])
    );
    assert_eq!(
        verdict(&manifest, &staged([300, 301], [310, 310])),
        Verdict::Fail(vec![noise("refusal-host-1", Lane::Open, "pids")])
    );
}

#[test]
fn a_reach_probe_whose_ruled_values_relate_differently_in_the_open_lane_discriminates() {
    let manifest = pid_rule("refusal-host-1", "pids");
    let mut runs = runs();
    never_discriminates(&mut runs, "refusal-host-1");
    for (lane, first, second) in [
        (Lane::Cell, 300, 300),
        (Lane::Open, 310, 312),
        (Lane::Absent, 320, 320),
    ] {
        for run in 0..2 {
            let fields = &mut observation(&mut runs, lane, run, "refusal-host-1").fields;
            fields.insert("pids".to_string(), json!([first + run, second + run]));
        }
    }
    assert_eq!(verdict(&manifest, &runs), Verdict::Pass);
}

#[test]
fn a_reach_probe_equal_in_one_pair_of_boots_does_not_discriminate() {
    for run in 0..2 {
        let mut runs = runs();
        let open = &mut observation(&mut runs, Lane::Open, run, "refusal-host-1").fields;
        open.insert("outcome".to_string(), json!("name does not resolve"));
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::Fail(vec![
                noise("refusal-host-1", Lane::Open, "outcome"),
                finding("refusal-host-1", Reason::NoDiscrimination),
            ]),
            "open run {run}"
        );
    }
}

#[test]
fn a_failure_in_a_completed_run_is_listed_when_the_lanes_other_run_is_not_run() {
    let mut runs = runs();
    let copied = json!({"source": "plasmosome-software", "fstype": "virtiofs", "options": "ro"});
    let mounts = &mut observation(&mut runs, Lane::Cell, 0, "hint-mounts").fields;
    mounts.insert("/opt".to_string(), copied);
    observation(&mut runs, Lane::Cell, 1, "hint-mounts").completed = false;
    never_discriminates(&mut runs, "refusal-host-3");
    observation(&mut runs, Lane::Open, 1, "refusal-host-3").completed = false;
    let transcript = runs[0].transcript.as_mut().expect("a transcript");
    transcript.insert("exit_statuses".to_string(), json!([1]));
    runs[1].transcript = None;
    let on_transcript = |reason| Finding {
        subject: Subject::Transcript,
        reason,
    };
    let name = NeutralName::Mount {
        path: "/opt".to_string(),
        key: MountKey::Source,
    };
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::NotRun {
            not_run: vec![
                not_run("refusal-host-3", Lane::Open),
                not_run("hint-mounts", Lane::Cell),
                on_transcript(Reason::NotRun { lane: Lane::Cell }),
            ],
            failures: vec![
                finding("refusal-host-3", Reason::NoDiscrimination),
                finding(
                    "hint-mounts",
                    Reason::NotNeutral {
                        lane: Lane::Cell,
                        name
                    }
                ),
                super::differs("hint-mounts", "/opt"),
                on_transcript(Reason::Differs {
                    field: "exit_statuses".to_string()
                }),
            ],
        }
    );
}

#[test]
fn every_completed_run_is_compared_with_every_completed_run_of_the_other_lane() {
    for swaps in 0..8 {
        let mut runs = runs();
        let hint = &mut observation(&mut runs, Lane::Cell, 0, "hint-env").fields;
        hint.insert("HOME".to_string(), json!("/home/plasmosome"));
        observation(&mut runs, Lane::Cell, 1, "hint-env").completed = false;
        observation(&mut runs, Lane::Absent, 0, "hint-env").completed = false;
        never_discriminates(&mut runs, "refusal-host-1");
        observation(&mut runs, Lane::Open, 1, "refusal-host-1").completed = false;
        observation(&mut runs, Lane::Absent, 0, "refusal-host-1").completed = false;
        for lane in Lane::ALL {
            if swaps & (1 << lane as usize) != 0 {
                runs.swap(2 * lane as usize, 2 * lane as usize + 1);
            }
        }
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::NotRun {
                not_run: vec![
                    not_run("refusal-host-1", Lane::Open),
                    not_run("refusal-host-1", Lane::Absent),
                    not_run("hint-env", Lane::Cell),
                    not_run("hint-env", Lane::Absent),
                ],
                failures: vec![
                    finding("refusal-host-1", Reason::NoDiscrimination),
                    super::differs("hint-env", "HOME"),
                ],
            },
            "runs swapped in the lanes {swaps:03b}"
        );
    }
}

#[test]
fn a_probe_whose_two_runs_differ_is_noise_in_that_lane() {
    for lane in Lane::ALL {
        for run in 0..2 {
            let mut runs = runs();
            observation(&mut runs, lane, run, "hint-cgroup")
                .fields
                .insert("controllers".to_string(), json!("cpu memory"));
            let mut expected = vec![noise("hint-cgroup", lane, "controllers")];
            if lane != Lane::Open {
                expected.push(super::differs("hint-cgroup", "controllers"));
            }
            assert_eq!(
                verdict(&manifest(), &runs),
                Verdict::Fail(expected),
                "{lane:?} run {run}"
            );
        }
    }
}

#[test]
fn a_step_failure_in_one_run_is_not_run_even_when_it_looks_like_absence() {
    let mut runs = runs();
    observation(&mut runs, Lane::Absent, 1, "refusal-host-1").failed_before_step = true;
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::NotRun {
            not_run: vec![not_run("refusal-host-1", Lane::Absent)],
            failures: vec![],
        }
    );
}

#[test]
fn a_plain_probe_that_failed_before_its_step_or_left_no_marker_is_not_run() {
    for no_marker in [false, true] {
        let mut runs = runs();
        for lane in Lane::ALL {
            for run in 0..2 {
                let seen = observation(&mut runs, lane, run, "hint-env");
                seen.completed = !no_marker;
                seen.failed_before_step = !no_marker;
            }
        }
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::NotRun {
                not_run: Lane::ALL.map(|lane| not_run("hint-env", lane)).to_vec(),
                failures: vec![],
            },
            "no marker: {no_marker}"
        );
    }
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
        (0, 3, Lane::Cell, Lane::Open),
        (1, 4, Lane::Cell, Lane::Absent),
        (3, 5, Lane::Open, Lane::Absent),
    ] {
        for observed_nothing in [false, true] {
            let mut runs = runs();
            runs[to].boot_id = runs[from].boot_id.clone();
            if observed_nothing {
                runs[to].observations.clear();
            }
            assert_eq!(
                judge(&manifest(), &runs),
                Err(RunRefusal::SameBoot {
                    boot_id: runs[from].boot_id.clone(),
                    first,
                    second
                }),
                "run {to} observed nothing: {observed_nothing}"
            );
        }
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
