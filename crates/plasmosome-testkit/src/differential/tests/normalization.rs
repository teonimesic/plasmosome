use serde_json::{Value, json};

use super::{
    differs, finding, load_value, manifest, manifest_json, object, observation, runs, set, verdict,
};
use crate::differential::normalize::normalize;
use crate::differential::{
    Lane, Manifest, Observation, Reason, Rule, RuleKind, Run, RunRefusal, Verdict, judge,
};

const TOKEN_A: &str = "4f2a9c0d1e3b5a7c9e0f1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f";
const TOKEN_B: &str = "9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b3a291807f6e5d4c3b2a1908f7e6d5c4b";

const WHOLE: [RuleKind; 7] = [
    RuleKind::Pid,
    RuleKind::MountId,
    RuleKind::ParentMountId,
    RuleKind::PeerGroup,
    RuleKind::Device,
    RuleKind::Inode,
    RuleKind::MemoryCounter,
];

const NUMBERS: [RuleKind; 3] = [
    RuleKind::SystemTime,
    RuleKind::Duration,
    RuleKind::LoadCounter,
];

const NAMES: [RuleKind; 9] = [
    RuleKind::Pid,
    RuleKind::BootId,
    RuleKind::MountId,
    RuleKind::ParentMountId,
    RuleKind::PeerGroup,
    RuleKind::Device,
    RuleKind::Inode,
    RuleKind::EphemeralPort,
    RuleKind::ResolvedAddress,
];

fn with_rules(probe: &str, rules: &[(&str, RuleKind)]) -> Manifest {
    let mut manifest = manifest();
    manifest
        .rules
        .extend(rules.iter().map(|&(field, kind)| Rule {
            probe: probe.to_string(),
            field: field.to_string(),
            kind,
        }));
    manifest
}

fn with_rule(probe: &str, field: &str, kind: RuleKind) -> Manifest {
    with_rules(probe, &[(field, kind)])
}

fn with_named_rules(probe: &str, rules: &[(&str, &str)]) -> Manifest {
    let mut manifest = manifest_json();
    let table = manifest["rules"].as_array_mut().expect("rules is a list");
    table.extend(
        rules
            .iter()
            .map(|(field, kind)| json!({"probe": probe, "field": field, "kind": kind})),
    );
    load_value(&manifest).expect("the manifest loads")
}

fn x_and_y(kinds: [RuleKind; 2], shown: impl Fn(Lane, u64) -> [Value; 2]) -> Verdict {
    let manifest = with_rules("hint-env", &[("x", kinds[0]), ("y", kinds[1])]);
    let mut runs = runs();
    for (index, field) in ["x", "y"].into_iter().enumerate() {
        each_run(&mut runs, "hint-env", field, |lane, run| {
            shown(lane, seed(lane, run))[index].clone()
        });
    }
    verdict(&manifest, &runs)
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

fn mixed_times(seed: u64, count: u64) -> Value {
    let mut state = seed;
    let mut offsets: Vec<u64> = (0..count).collect();
    for index in (1..offsets.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        offsets.swap(index, ((state >> 33) % (index as u64 + 1)) as usize);
    }
    let time = |offset: &u64| match offset % 2 {
        0 => json!(((1_u64 << 53) + offset) as f64),
        _ => json!((1_u64 << 53) + offset),
    };
    json!(offsets.iter().map(time).collect::<Vec<Value>>())
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
        Verdict::Fail(vec![
            differs("op-inodes", "distinct"),
            differs("op-inodes", "linked")
        ])
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
fn an_extra_process_in_one_lane_changes_no_other_probes_placeholders() {
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
fn a_leader_that_names_another_process_of_its_list_fails() {
    let mut runs = runs();
    for run in 0..2 {
        let processes = &mut observation(&mut runs, Lane::Cell, run, "hint-processes").fields;
        processes.insert("leader".to_string(), json!(100 + run));
    }
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![differs("hint-processes", "pids")])
    );
}

#[test]
fn a_leader_relates_to_its_list_across_a_field_of_another_name_kind() {
    let manifest = with_rule("hint-processes", "mounts", RuleKind::MountId);
    let mut runs = runs();
    each_run(&mut runs, "hint-processes", "mounts", |lane, run| {
        json!([20 + seed(lane, run)])
    });
    for run in 0..2 {
        let processes = &mut observation(&mut runs, Lane::Cell, run, "hint-processes").fields;
        processes.insert("leader".to_string(), json!(100 + run));
    }
    assert_eq!(
        verdict(&manifest, &runs),
        Verdict::Fail(vec![differs("hint-processes", "pids")])
    );
}

#[test]
fn fields_are_taken_in_byte_order_so_an_upper_case_name_sorts_first() {
    let manifest = with_rules(
        "hint-env",
        &[("Pid", RuleKind::Pid), ("PPid", RuleKind::Pid)],
    );
    let mut runs = runs();
    each_run(&mut runs, "hint-env", "Pid", |lane, run| {
        json!(200 + seed(lane, run))
    });
    each_run(&mut runs, "hint-env", "PPid", |lane, run| match lane {
        Lane::Cell => json!(200 + seed(lane, run)),
        _ => json!(100 + seed(lane, run)),
    });
    assert_eq!(
        verdict(&manifest, &runs),
        Verdict::Fail(vec![differs("hint-env", "Pid")])
    );
}

#[test]
fn two_files_that_report_one_inode_in_two_fields_fail_and_a_hard_linked_pair_passes() {
    let manifest = with_rules(
        "op-inodes",
        &[("a", RuleKind::Inode), ("b", RuleKind::Inode)],
    );
    let fail = Verdict::Fail(vec![differs("op-inodes", "b")]);
    for (others, cell, wanted) in [
        ([500, 600], [500, 600], &Verdict::Pass),
        ([500, 600], [500, 500], &fail),
        ([700, 700], [700, 700], &Verdict::Pass),
        ([700, 700], [700, 701], &fail),
    ] {
        let mut runs = runs();
        for lane in Lane::ALL {
            for run in 0..2 {
                let [a, b] = match lane {
                    Lane::Cell => cell,
                    _ => others,
                }
                .map(|inode| inode + seed(lane, run));
                observation(&mut runs, lane, run, "op-inodes").fields =
                    object(json!({"a": a, "b": b}));
            }
        }
        assert_eq!(
            &verdict(&manifest, &runs),
            wanted,
            "cell {cell:?}, others {others:?}"
        );
    }
}

#[test]
fn a_parent_mount_id_is_numbered_with_the_mount_ids() {
    for (parents, shown_in) in [("parents", "parents"), ("a_parents", "mount_ids")] {
        let manifest = with_rules(
            "hint-env",
            &[
                (parents, RuleKind::ParentMountId),
                ("mount_ids", RuleKind::MountId),
            ],
        );
        for (cell_parent, wanted) in [
            (20, Verdict::Pass),
            (21, Verdict::Fail(vec![differs("hint-env", shown_in)])),
        ] {
            let mut runs = runs();
            each_run(&mut runs, "hint-env", "mount_ids", |lane, run| {
                json!([20 + seed(lane, run), 21 + seed(lane, run)])
            });
            each_run(&mut runs, "hint-env", parents, |lane, run| {
                let parent = match lane {
                    Lane::Cell => cell_parent,
                    _ => 20,
                };
                json!([1, parent + seed(lane, run)])
            });
            assert_eq!(
                verdict(&manifest, &runs),
                wanted,
                "{parents} in the cell names mount {cell_parent}"
            );
        }
    }
}

#[test]
fn peer_groups_relate_across_fields_in_byte_order() {
    let manifest = with_named_rules(
        "hint-env",
        &[("masters", "peer_group"), ("a_shared", "peer_group")],
    );
    for (cell_shared, wanted) in [
        (41, Verdict::Pass),
        (40, Verdict::Fail(vec![differs("hint-env", "masters")])),
    ] {
        let mut runs = runs();
        each_run(&mut runs, "hint-env", "masters", |lane, run| {
            json!([40 + seed(lane, run), 41 + seed(lane, run)])
        });
        each_run(&mut runs, "hint-env", "a_shared", |lane, run| {
            let shared = match lane {
                Lane::Cell => cell_shared,
                _ => 41,
            };
            json!([shared + seed(lane, run)])
        });
        assert_eq!(
            verdict(&manifest, &runs),
            wanted,
            "a_shared in the cell names group {cell_shared}"
        );
    }
}

#[test]
fn a_peer_group_never_shares_numbers_with_mount_ids() {
    let manifest = with_named_rules(
        "hint-env",
        &[
            ("mounts", "mount_id"),
            ("parents", "parent_mount_id"),
            ("groups", "peer_group"),
        ],
    );
    for cell_group in [50, 51, 52] {
        let mut runs = runs();
        each_run(&mut runs, "hint-env", "mounts", |lane, run| {
            json!([50 + seed(lane, run)])
        });
        each_run(&mut runs, "hint-env", "parents", |lane, run| {
            json!([51 + seed(lane, run)])
        });
        each_run(&mut runs, "hint-env", "groups", |lane, run| {
            let group = match lane {
                Lane::Cell => cell_group,
                _ => 52,
            };
            json!([group + seed(lane, run)])
        });
        assert_eq!(
            verdict(&manifest, &runs),
            Verdict::Pass,
            "groups in the cell holds {cell_group}"
        );
    }
}

#[test]
fn two_kinds_that_name_things_are_never_numbered_together() {
    let numbered = [
        RuleKind::Pid,
        RuleKind::MountId,
        RuleKind::ParentMountId,
        RuleKind::PeerGroup,
        RuleKind::Device,
        RuleKind::Inode,
        RuleKind::EphemeralPort,
    ];
    let mounts = [RuleKind::MountId, RuleKind::ParentMountId];
    for first in numbered {
        for second in numbered {
            if first == second || (mounts.contains(&first) && mounts.contains(&second)) {
                continue;
            }
            let shown = |lane, seed| match lane {
                Lane::Cell => [json!(500 + seed), json!(500 + seed)],
                _ => [json!(500 + seed), json!(600 + seed)],
            };
            assert_eq!(
                x_and_y([first, second], shown),
                Verdict::Pass,
                "{} and {}",
                first.name(),
                second.name()
            );
        }
    }
}

#[test]
fn a_boot_id_or_a_resolved_address_shares_no_numbers_with_another_name_kind() {
    for kind in [RuleKind::BootId, RuleKind::ResolvedAddress] {
        for other in NAMES.into_iter().filter(|other| *other != kind) {
            let manifest = with_rules("hint-env", &[("a_other", other), (kind.name(), kind)]);
            let mut runs = runs();
            each_run(&mut runs, "hint-env", "a_other", |lane, run| match lane {
                Lane::Cell => json!([sample(other, 1), sample(other, 100 + seed(lane, run))]),
                _ => json!([sample(other, 1)]),
            });
            each_run(&mut runs, "hint-env", kind.name(), |lane, run| {
                sample(kind, seed(lane, run))
            });
            assert_eq!(
                verdict(&manifest, &runs),
                Verdict::Fail(vec![differs("hint-env", "a_other")]),
                "{} beside {}",
                kind.name(),
                other.name()
            );
        }
    }
}

#[test]
fn every_kind_that_names_things_relates_its_values_across_fields() {
    for kind in NAMES {
        let apart = |_, seed| [sample(kind, seed), sample(kind, seed + 1)];
        assert_eq!(
            x_and_y([kind, kind], apart),
            Verdict::Pass,
            "{}",
            kind.name()
        );
        let joined = |lane, seed| match lane {
            Lane::Cell => [sample(kind, seed), sample(kind, seed)],
            _ => apart(lane, seed),
        };
        assert_eq!(
            x_and_y([kind, kind], joined),
            Verdict::Fail(vec![differs("hint-env", "y")]),
            "{}",
            kind.name()
        );
    }
}

#[test]
fn times_counters_and_durations_in_two_fields_are_not_related() {
    for kind in [
        RuleKind::SystemTime,
        RuleKind::MemoryCounter,
        RuleKind::LoadCounter,
        RuleKind::Duration,
    ] {
        for cell in [[5, 5], [6, 5]] {
            let shown = |lane, _| match lane {
                Lane::Cell => cell.map(|value| json!(value)),
                _ => [json!(5), json!(6)],
            };
            assert_eq!(
                x_and_y([kind, kind], shown),
                Verdict::Pass,
                "{} cell {cell:?}",
                kind.name()
            );
        }
    }
}

#[test]
fn durations_keep_no_relation() {
    let manifest = with_rule("hint-env", "elapsed", RuleKind::Duration);
    let fail = Verdict::Fail(vec![differs("hint-env", "elapsed")]);
    for (cell, wanted) in [
        (json!([0.5, 1.5, 1.5]), &Verdict::Pass),
        (json!([0.5, 1.5, 2.5]), &Verdict::Pass),
        (json!([1.5, 0.5, 1.5]), &Verdict::Pass),
        (json!([0.5, 1.5]), &fail),
    ] {
        let mut runs = runs();
        each_run(&mut runs, "hint-env", "elapsed", |lane, _| match lane {
            Lane::Cell => cell.clone(),
            _ => json!([0.5, 1.5, 0.5]),
        });
        assert_eq!(&verdict(&manifest, &runs), wanted, "cell {cell}");
    }
    let mut runs = runs();
    each_run(&mut runs, "hint-env", "elapsed", |_, _| json!([5, 5.0]));
    assert_eq!(judge(&manifest, &runs), Ok(Verdict::Pass));
}

#[test]
fn a_reach_probe_whose_lanes_differ_only_in_their_durations_does_not_discriminate() {
    let manifest = with_rule("refusal-host-1", "elapsed", RuleKind::Duration);
    for (open, others) in [([10, 20], [20, 10]), ([10, 10], [10, 20])] {
        let mut runs = runs();
        let refused = json!("name does not resolve");
        set(&mut runs, Lane::Open, "refusal-host-1", "outcome", refused);
        each_run(&mut runs, "refusal-host-1", "elapsed", |lane, run| {
            let shown = match lane {
                Lane::Open => open,
                _ => others,
            };
            json!(shown.map(|elapsed| elapsed + seed(lane, run)))
        });
        assert_eq!(
            verdict(&manifest, &runs),
            Verdict::Fail(vec![finding("refusal-host-1", Reason::NoDiscrimination)]),
            "open {open:?}, others {others:?}"
        );
    }
}

#[test]
fn placeholders_name_their_kind_and_names_share_numbers_across_fields() {
    let manifest = with_rules(
        "hint-env",
        &[
            ("z_parents", RuleKind::ParentMountId),
            ("mounts", RuleKind::MountId),
            ("elapsed", RuleKind::Duration),
            ("times", RuleKind::SystemTime),
            ("rss", RuleKind::MemoryCounter),
        ],
    );
    let observation = Observation {
        probe: "hint-env".to_string(),
        completed: true,
        failed_before_step: false,
        fields: object(json!({
            "mounts": [20, 21],
            "z_parents": [1, 20],
            "elapsed": [7, 3],
            "times": [30, 10, 30, 40],
            "rss": [5, 5, 6],
            "other": 4
        })),
    };
    assert_eq!(
        normalize(&manifest, Lane::Cell, &observation),
        Ok(object(json!({
            "mounts": ["<mount_id:1>", "<mount_id:2>"],
            "z_parents": ["<mount_id:3>", "<mount_id:1>"],
            "elapsed": ["<duration>", "<duration>"],
            "times": [
                "<system_time:2>",
                "<system_time:1>",
                "<system_time:2>",
                "<system_time:3>"
            ],
            "rss": ["<memory_counter:1>", "<memory_counter:1>", "<memory_counter:2>"],
            "other": 4
        })))
    );
}

#[test]
fn peer_group_placeholders_name_their_kind_and_keep_first_appearance() {
    let manifest = with_named_rules(
        "hint-env",
        &[
            ("masters", "peer_group"),
            ("shared", "peer_group"),
            ("mounts", "mount_id"),
        ],
    );
    let observation = Observation {
        probe: "hint-env".to_string(),
        completed: true,
        failed_before_step: false,
        fields: object(json!({"masters": [9, 7, 8, 9], "shared": [8, 7], "mounts": [7]})),
    };
    assert_eq!(
        normalize(&manifest, Lane::Cell, &observation),
        Ok(object(json!({
            "masters": [
                "<peer_group:1>",
                "<peer_group:2>",
                "<peer_group:3>",
                "<peer_group:1>"
            ],
            "shared": ["<peer_group:3>", "<peer_group:2>"],
            "mounts": ["<mount_id:1>"]
        })))
    );
}

#[test]
fn time_placeholders_keep_the_order_of_the_times() {
    let late: u64 = 1_760_954_400_000_000_001;
    let fail = Verdict::Fail(vec![differs("op-times", "times")]);
    for (cell, wanted) in [
        ([json!([3, 5]), json!([13, 15])], &Verdict::Pass),
        ([json!([5, 3]), json!([15, 13])], &fail),
        ([json!([5, 5]), json!([15, 15])], &fail),
        ([json!([0.5, 0.25]), json!([0.75, 0.5])], &fail),
        ([json!([-1, -2]), json!([-11, -12])], &fail),
        ([json!([late, late - 1]), json!([late + 1, late])], &fail),
        (
            [
                json!([u64::MAX - 3, u64::MAX - 2]),
                json!([u64::MAX - 1, u64::MAX]),
            ],
            &Verdict::Pass,
        ),
        (
            [
                json!([u64::MAX - 2, u64::MAX - 3]),
                json!([u64::MAX, u64::MAX - 1]),
            ],
            &fail,
        ),
        (
            [json!([i64::MIN, u64::MAX]), json!([-1, 0])],
            &Verdict::Pass,
        ),
        ([json!([u64::MAX, i64::MIN]), json!([0, -1])], &fail),
        (
            [
                json!([i64::MIN + 1, i64::MIN]),
                json!([i64::MIN + 3, i64::MIN + 2]),
            ],
            &fail,
        ),
        ([json!([-0.5, 0.25]), json!([-0.25, 0.5])], &Verdict::Pass),
        ([json!([-0.0, 0.0]), json!([-0.0, 0.0])], &fail),
    ] {
        let mut runs = runs();
        for (run, times) in cell.iter().enumerate() {
            let fields = &mut observation(&mut runs, Lane::Cell, run, "op-times").fields;
            fields.insert("times".to_string(), times.clone());
        }
        assert_eq!(&verdict(&manifest(), &runs), wanted, "{cell:?}");
    }
}

#[test]
fn every_kind_but_durations_keeps_equality_in_a_field_and_only_system_times_keep_order() {
    for kind in RuleKind::ALL {
        let manifest = with_rule("hint-env", "ruled", kind);
        let fail = || match kind {
            RuleKind::Duration => Verdict::Pass,
            _ => Verdict::Fail(vec![differs("hint-env", "ruled")]),
        };
        let reordered = match kind {
            RuleKind::SystemTime => fail(),
            _ => Verdict::Pass,
        };
        for (others, cell, wanted) in [
            ([0, 1, 0], [0, 1, 0], Verdict::Pass),
            ([0, 1, 0], [0, 1, 1], fail()),
            ([0, 1, 0], [0, 1, 2], fail()),
            ([0, 1, 0], [1, 0, 1], reordered),
            ([0, 1, 2], [0, 1, 1], fail()),
        ] {
            let mut runs = runs();
            let base = ruled_list(kind, others);
            let shown = ruled_list(kind, cell);
            each_run(&mut runs, "hint-env", "ruled", |lane, run| match lane {
                Lane::Cell => shown(lane, run),
                _ => base(lane, run),
            });
            assert_eq!(
                verdict(&manifest, &runs),
                wanted,
                "{} cell {cell:?}, others {others:?}",
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
            for shown in [value.clone(), json!([value, value])] {
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
fn an_empty_list_in_a_ruled_field_is_held_by_every_kind() {
    for kind in RuleKind::ALL {
        let manifest = with_rule("hint-env", "ruled", kind);
        let mut runs = runs();
        each_run(&mut runs, "hint-env", "ruled", |_, _| json!([]));
        assert_eq!(
            judge(&manifest, &runs),
            Ok(Verdict::Pass),
            "{}",
            kind.name()
        );
    }
}

#[test]
fn a_system_time_list_that_mixes_whole_numbers_and_fractions_is_refused() {
    let text = |json: &str| serde_json::from_str::<Value>(json).expect("JSON text");
    for (times, lane, run) in [
        (
            json!([9_007_199_254_740_993_u64, 9_007_199_254_740_992.0]),
            Lane::Cell,
            0,
        ),
        (json!([5, 5.0]), Lane::Open, 1),
        (mixed_times(9, 24), Lane::Absent, 0),
        (mixed_times(0, 1_000), Lane::Cell, 1),
        (json!([u64::MAX, 0.5]), Lane::Absent, 1),
        (text("[1e3, 1001]"), Lane::Open, 0),
        (text("[-0, 5]"), Lane::Cell, 1),
    ] {
        let mut runs = runs();
        let fields = &mut observation(&mut runs, lane, run, "op-times").fields;
        fields.insert("times".to_string(), times.clone());
        let length = times.as_array().map_or(0, Vec::len);
        assert_eq!(
            judge(&manifest(), &runs),
            Err(RunRefusal::RuleCannotHold {
                lane,
                probe: "op-times".to_string(),
                field: "times".to_string(),
                kind: RuleKind::SystemTime
            }),
            "{length} times in {lane:?} run {run}"
        );
    }
    for kind in [RuleKind::LoadCounter, RuleKind::Duration] {
        let manifest = with_rule("hint-env", "ruled", kind);
        let mut runs = runs();
        each_run(&mut runs, "hint-env", "ruled", |_, _| json!([5, 5.0]));
        assert_eq!(
            judge(&manifest, &runs),
            Ok(Verdict::Pass),
            "{}",
            kind.name()
        );
    }
}

#[test]
fn a_whole_number_rule_holds_no_negative_fraction_text_or_token() {
    refused_wherever_it_is(
        &WHOLE,
        &[json!(0), json!(4_242), json!(u64::MAX)],
        &[
            json!(-1),
            json!(1.5),
            json!(300.0),
            json!("4242"),
            json!(TOKEN_A),
        ],
    );
}

#[test]
fn a_peer_group_rule_holds_only_whole_numbers() {
    refused_wherever_it_is(
        &[RuleKind::PeerGroup],
        &[json!(0), json!(u64::MAX)],
        &[
            json!("master:5"),
            json!("shared:5"),
            json!("propagate_from:5"),
            json!(-5),
            json!(5.5),
        ],
    );
}

#[test]
fn an_ephemeral_port_rule_holds_only_whole_numbers_up_to_65535() {
    refused_wherever_it_is(
        &[RuleKind::EphemeralPort],
        &[json!(0), json!(49_152), json!(65_535)],
        &[
            json!(65_536),
            json!(-1),
            json!(443.5),
            json!(443.0),
            json!("443"),
        ],
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
            json!("6e1f3a0g-9b2d-4c5e-8f7a-1b2c3d4e5f60"),
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
