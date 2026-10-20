use serde_json::{Value, json};

use super::{differs, finding, manifest, observation, runs, set, verdict};
use crate::differential::{
    Lane, Manifest, Reason, Rule, RuleKind, Run, RunRefusal, Verdict, judge,
};

const TOKEN_A: &str = "4f2a9c0d1e3b5a7c9e0f1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f";
const TOKEN_B: &str = "9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b3a291807f6e5d4c3b2a1908f7e6d5c4b";

const WHOLE: [RuleKind; 6] = [
    RuleKind::Pid,
    RuleKind::MountId,
    RuleKind::ParentMountId,
    RuleKind::Device,
    RuleKind::Inode,
    RuleKind::MemoryCounter,
];

const NUMBERS: [RuleKind; 3] = [
    RuleKind::SystemTime,
    RuleKind::Duration,
    RuleKind::LoadCounter,
];

fn with_rule(probe: &str, field: &str, kind: RuleKind) -> Manifest {
    let mut manifest = manifest();
    manifest.rules.push(Rule {
        probe: probe.to_string(),
        field: field.to_string(),
        kind,
    });
    manifest
}

fn each_run(runs: &mut [Run], probe: &str, field: &str, value: impl Fn(Lane, usize) -> Value) {
    for lane in Lane::ALL {
        for run in 0..2 {
            let fields = &mut observation(runs, lane, run, probe).fields;
            fields.insert(field.to_string(), value(lane, run));
        }
    }
}

fn seed(lane: Lane, run: usize) -> u64 {
    10 * lane as u64 + 3 * run as u64
}

fn sample(kind: RuleKind, n: u64) -> Value {
    match kind {
        RuleKind::EphemeralPort => json!(30_000 + n),
        RuleKind::SystemTime | RuleKind::Duration | RuleKind::LoadCounter => {
            json!(n as f64 + 0.5)
        }
        RuleKind::BootId => json!(format!("6e1f3a0c-9b2d-4c5e-8f7a-{n:012x}")),
        RuleKind::ResolvedAddress => json!(format!("198.18.0.{n}")),
        _ => json!(1_000 + n),
    }
}

fn ruled_list(kind: RuleKind, order: [u64; 3]) -> impl Fn(Lane, usize) -> Value {
    move |lane, run| json!(order.map(|offset| sample(kind, seed(lane, run) + offset)))
}

#[test]
fn a_64_hex_environment_value_is_compared_exactly_and_fails() {
    let mut runs = runs();
    set(&mut runs, Lane::Cell, "hint-env", "SESSION", json!(TOKEN_A));
    set(&mut runs, Lane::Open, "hint-env", "SESSION", json!(TOKEN_B));
    set(
        &mut runs,
        Lane::Absent,
        "hint-env",
        "SESSION",
        json!(TOKEN_B),
    );
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![differs("hint-env", "SESSION")])
    );
}

#[test]
fn a_value_that_looks_like_a_rule_kind_is_compared_exactly_where_no_rule_covers_it() {
    for (cell, other) in [
        (json!(4_242), json!(4_243)),
        (json!(0.25), json!(0.5)),
        (
            json!("6e1f3a0c-9b2d-4c5e-8f7a-1b2c3d4e5f60"),
            json!("6e1f3a0c-9b2d-4c5e-8f7a-1b2c3d4e5f61"),
        ),
        (json!("198.18.0.1"), json!("198.18.0.2")),
        (json!([300, 301]), json!([310, 311])),
    ] {
        let mut runs = runs();
        for lane in Lane::ALL {
            let value = if lane == Lane::Cell { &cell } else { &other };
            set(&mut runs, lane, "hint-self", "parent", value.clone());
        }
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::Fail(vec![differs("hint-self", "parent")]),
            "{cell} against {other}"
        );
    }
}

#[test]
fn a_time_the_workload_set_is_compared_exactly_even_beside_a_rule_on_that_field_elsewhere() {
    let elsewhere = with_rule("op-times", "mtime", RuleKind::SystemTime);
    for manifest in [manifest(), elsewhere] {
        let mut runs = runs();
        set(
            &mut runs,
            Lane::Cell,
            "op-utimensat",
            "mtime",
            json!(1_760_954_400),
        );
        assert_eq!(
            verdict(&manifest, &runs),
            Verdict::Fail(vec![differs("op-utimensat", "mtime")]),
            "{} rules",
            manifest.rules.len()
        );
    }
}

#[test]
fn two_distinct_files_that_report_one_inode_fail() {
    let mut runs = runs();
    for run in 0..2 {
        let inodes = &mut observation(&mut runs, Lane::Cell, run, "op-inodes").fields;
        inodes.insert("distinct".to_string(), json!([5 + run, 5 + run]));
    }
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![differs("op-inodes", "distinct")])
    );
}

#[test]
fn a_hard_linked_pair_compares_equal_and_a_broken_one_fails() {
    for (second, wanted) in [
        (0, Verdict::Pass),
        (1, Verdict::Fail(vec![differs("op-inodes", "linked")])),
    ] {
        let mut runs = runs();
        for run in 0..2 {
            let inodes = &mut observation(&mut runs, Lane::Cell, run, "op-inodes").fields;
            let first = 777 + 5 * run;
            inodes.insert("linked".to_string(), json!([first, first + second]));
        }
        assert_eq!(
            verdict(&manifest(), &runs),
            wanted,
            "second inode +{second}"
        );
    }
}

#[test]
fn an_extra_process_in_one_lane_changes_no_other_placeholder() {
    let mut runs = runs();
    for run in 0..2 {
        let processes = &mut observation(&mut runs, Lane::Cell, run, "hint-processes").fields;
        let pids = json!([1, 100 + run, 150 + run, 200 + run]);
        processes.insert("pids".to_string(), pids);
    }
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![differs("hint-processes", "pids")])
    );
}

#[test]
fn time_placeholders_keep_the_order_of_the_times() {
    let late: u64 = 1_760_954_400_000_000_001;
    let fail = Verdict::Fail(vec![differs("op-times", "times")]);
    for kind in [RuleKind::SystemTime, RuleKind::Duration] {
        let mut manifest = manifest();
        manifest.rules[5].kind = kind;
        for (cell, wanted) in [
            ([json!([3, 5]), json!([13, 15])], &Verdict::Pass),
            ([json!([5, 3]), json!([15, 13])], &fail),
            ([json!([5, 5]), json!([15, 15])], &fail),
            ([json!([0.5, 0.25]), json!([0.75, 0.5])], &fail),
            ([json!([-1, -2]), json!([-11, -12])], &fail),
            ([json!([late, late - 1]), json!([late + 1, late])], &fail),
        ] {
            let mut runs = runs();
            for (run, times) in cell.iter().enumerate() {
                let fields = &mut observation(&mut runs, Lane::Cell, run, "op-times").fields;
                fields.insert("times".to_string(), times.clone());
            }
            assert_eq!(
                &verdict(&manifest, &runs),
                wanted,
                "{} {cell:?}",
                kind.name()
            );
        }
    }
}

#[test]
fn every_rule_kind_keeps_equality_and_only_times_keep_order() {
    for kind in RuleKind::ALL {
        let manifest = with_rule("hint-env", "ruled", kind);
        let ordered = matches!(kind, RuleKind::SystemTime | RuleKind::Duration);
        for (cell, wanted) in [
            ([0, 1, 0], Verdict::Pass),
            ([0, 1, 1], Verdict::Fail(vec![differs("hint-env", "ruled")])),
            (
                [1, 0, 1],
                match ordered {
                    true => Verdict::Fail(vec![differs("hint-env", "ruled")]),
                    false => Verdict::Pass,
                },
            ),
        ] {
            let mut runs = runs();
            let base = ruled_list(kind, [0, 1, 0]);
            let shown = ruled_list(kind, cell);
            each_run(&mut runs, "hint-env", "ruled", |lane, run| match lane {
                Lane::Cell => shown(lane, run),
                _ => base(lane, run),
            });
            assert_eq!(
                verdict(&manifest, &runs),
                wanted,
                "{} cell {cell:?}",
                kind.name()
            );
        }
    }
}

fn refused_wherever_it_is(kinds: &[RuleKind], held: &[Value], refused: &[Value]) {
    let shapes = [
        Value::Null,
        json!(true),
        json!({"pid": 1}),
        json!([[held[0]]]),
    ];
    for &kind in kinds {
        let manifest = with_rule("hint-env", "ruled", kind);
        for value in held {
            for shown in [value.clone(), json!([value, held[0]])] {
                let mut runs = runs();
                set(&mut runs, Lane::Cell, "hint-env", "ruled", shown.clone());
                set(&mut runs, Lane::Open, "hint-env", "ruled", shown.clone());
                set(&mut runs, Lane::Absent, "hint-env", "ruled", shown.clone());
                assert_eq!(
                    judge(&manifest, &runs),
                    Ok(Verdict::Pass),
                    "{} holds {shown}",
                    kind.name()
                );
            }
        }
        for value in refused.iter().chain(&shapes) {
            for shown in [value.clone(), json!([held[0], value])] {
                for lane in Lane::ALL {
                    for run in 0..2 {
                        let mut runs = runs();
                        each_run(&mut runs, "hint-env", "ruled", |_, _| held[0].clone());
                        let fields = &mut observation(&mut runs, lane, run, "hint-env").fields;
                        fields.insert("ruled".to_string(), shown.clone());
                        assert_eq!(
                            judge(&manifest, &runs),
                            Err(RunRefusal::RuleCannotHold {
                                lane,
                                probe: "hint-env".to_string(),
                                field: "ruled".to_string(),
                                kind
                            }),
                            "{} refuses {shown} in {lane:?} run {run}",
                            kind.name()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_whole_number_rule_holds_no_negative_fraction_text_or_token() {
    refused_wherever_it_is(
        &WHOLE,
        &[json!(0), json!(4_242), json!(u64::MAX)],
        &[json!(-1), json!(1.5), json!("4242"), json!(TOKEN_A)],
    );
}

#[test]
fn an_ephemeral_port_rule_holds_only_whole_numbers_up_to_65535() {
    refused_wherever_it_is(
        &[RuleKind::EphemeralPort],
        &[json!(0), json!(49_152), json!(65_535)],
        &[json!(65_536), json!(-1), json!(443.5), json!("443")],
    );
}

#[test]
fn a_time_duration_or_load_rule_holds_any_number_and_no_text() {
    refused_wherever_it_is(
        &NUMBERS,
        &[json!(0), json!(-1), json!(1.5), json!(u64::MAX)],
        &[json!("2026-10-20T10:00:00Z"), json!("4242")],
    );
}

#[test]
fn a_boot_id_rule_holds_only_a_uuid() {
    refused_wherever_it_is(
        &[RuleKind::BootId],
        &[
            json!("6e1f3a0c-9b2d-4c5e-8f7a-1b2c3d4e5f60"),
            json!("6E1F3A0C-9B2D-4C5E-8F7A-1B2C3D4E5F60"),
        ],
        &[
            json!(TOKEN_A),
            json!("6e1f3a0c9b2d4c5e8f7a1b2c3d4e5f60"),
            json!("6e1f3a0c-9b2d-4c5e-8f7a-1b2c3d4e5f6g"),
            json!("6e1f3a0c-9b2d-4c5e-8f7a1-b2c3d4e5f60"),
            json!("6e1f3a0c-9b2d-4c5e-8f7a-1b2c3d4e5f60-0"),
            json!(4_242),
        ],
    );
}

#[test]
fn a_resolved_address_rule_holds_only_an_ip_address() {
    refused_wherever_it_is(
        &[RuleKind::ResolvedAddress],
        &[json!("198.18.0.1"), json!("::1"), json!("2001:db8::7")],
        &[
            json!("example.test"),
            json!("198.18.0.1:443"),
            json!("198.18.0.256"),
            json!(4_242),
        ],
    );
}

#[test]
fn a_ruled_field_holding_an_object_is_refused_so_no_workload_set_value_hides_in_it() {
    let mut manifest = manifest();
    let times = manifest
        .probes
        .iter_mut()
        .find(|probe| probe.id == "op-times");
    times.expect("declared").workload_set = vec!["mtime".to_string()];
    let mut runs = runs();
    for (lane, mtime) in [(Lane::Cell, 0), (Lane::Open, 7), (Lane::Absent, 7)] {
        set(
            &mut runs,
            lane,
            "op-times",
            "times",
            json!({"mtime": mtime}),
        );
    }
    assert_eq!(
        judge(&manifest, &runs),
        Err(RunRefusal::RuleCannotHold {
            lane: Lane::Cell,
            probe: "op-times".to_string(),
            field: "times".to_string(),
            kind: RuleKind::SystemTime
        })
    );
}

#[test]
fn a_ruled_value_in_an_observation_that_did_not_complete_is_not_checked() {
    for failed_before_step in [false, true] {
        let mut runs = runs();
        let crashed = observation(&mut runs, Lane::Open, 1, "hint-self");
        crashed.completed = failed_before_step;
        crashed.failed_before_step = failed_before_step;
        crashed.fields.insert("pid".to_string(), json!(TOKEN_A));
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::NotRun {
                not_run: vec![finding("hint-self", Reason::NotRun { lane: Lane::Open })],
                failures: vec![],
            },
            "failed before its step: {failed_before_step}"
        );
    }
}
