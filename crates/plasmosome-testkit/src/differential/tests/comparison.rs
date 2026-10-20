use serde_json::{Value, json};

use super::{differs, finding, manifest, observation, runs, set, verdict};
use crate::differential::{Finding, Lane, MountKey, NeutralName, Reason, Subject, Verdict};

fn not_neutral(lane: Lane, path: &str, key: MountKey) -> Finding {
    let path = path.to_string();
    let name = NeutralName::Mount { path, key };
    finding("hint-mounts", Reason::NotNeutral { lane, name })
}

#[test]
fn an_allowance_admits_its_exact_value_and_no_other_in_its_field() {
    for other in [
        Some(json!("0640")),
        Some(json!(384)),
        Some(Value::Null),
        None,
    ] {
        let mut runs = runs();
        for run in 0..2 {
            let fields = &mut observation(&mut runs, Lane::Cell, run, "op-utimensat").fields;
            match &other {
                Some(value) => fields.insert("mode".to_string(), value.clone()),
                None => fields.remove("mode"),
            };
        }
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::Fail(vec![differs("op-utimensat", "mode")]),
            "{other:?}"
        );
    }
}

#[test]
fn an_allowance_never_excuses_a_new_field_or_another_field_in_its_probe() {
    let mut runs = runs();
    let xattr = json!("user.membrane");
    set(&mut runs, Lane::Cell, "op-utimensat", "xattr", xattr);
    set(&mut runs, Lane::Cell, "op-utimensat", "mtime", json!(0));
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![
            differs("op-utimensat", "mtime"),
            differs("op-utimensat", "xattr"),
        ])
    );
}

#[test]
fn an_allowance_admits_its_value_only_in_the_cell_lane() {
    let mut runs = runs();
    set(
        &mut runs,
        Lane::Absent,
        "op-utimensat",
        "mode",
        json!("0600"),
    );
    set(&mut runs, Lane::Cell, "op-utimensat", "mode", json!("0644"));
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![differs("op-utimensat", "mode")])
    );
}

#[test]
fn an_allowance_admits_its_value_only_in_its_own_probe() {
    let mut runs = runs();
    for lane in Lane::ALL {
        let mode = if lane == Lane::Cell { "0600" } else { "0644" };
        set(&mut runs, lane, "hint-self", "mode", json!(mode));
    }
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![differs("hint-self", "mode")])
    );
}

#[test]
fn the_transcript_admits_no_allowance() {
    let mut runs = runs();
    for run in &mut runs {
        let mode = if run.lane == Lane::Cell {
            "0600"
        } else {
            "0644"
        };
        let transcript = run.transcript.as_mut().expect("a transcript");
        transcript.insert("mode".to_string(), json!(mode));
    }
    let field = "mode".to_string();
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![Finding {
            subject: Subject::Transcript,
            reason: Reason::Differs { field },
        }])
    );
}

#[test]
fn a_cell_and_an_absent_lane_sharing_a_non_neutral_opt_source_both_fail() {
    let mut runs = runs();
    let copied = json!({"source": "plasmosome-software", "fstype": "virtiofs", "options": "ro"});
    set(&mut runs, Lane::Cell, "hint-mounts", "/opt", copied.clone());
    set(&mut runs, Lane::Absent, "hint-mounts", "/opt", copied);
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![
            not_neutral(Lane::Cell, "/opt", MountKey::Source),
            not_neutral(Lane::Absent, "/opt", MountKey::Source),
        ])
    );
}

#[test]
fn every_neutral_mount_name_is_checked_exactly_in_every_lane() {
    for (path, key, name, neutral) in [
        ("/usr/local/bin", "fstype", MountKey::Fstype, "virtiofs"),
        ("/opt", "options", MountKey::Options, "ro"),
        ("/opt", "source", MountKey::Source, "none"),
        ("/workspace", "subtype", MountKey::Subtype, "passthrough"),
        ("/workspace", "source", MountKey::Source, "workspace"),
    ] {
        let extended = format!("{neutral},plasmosome");
        for shown in [Some("plasmosome"), Some(extended.as_str()), None] {
            let mut runs = runs();
            for lane in Lane::ALL {
                for run in 0..2 {
                    let fields = &mut observation(&mut runs, lane, run, "hint-mounts").fields;
                    let mount = fields[path].as_object_mut().expect("a mount");
                    match shown {
                        Some(value) => mount.insert(key.to_string(), json!(value)),
                        None => mount.remove(key),
                    };
                }
            }
            assert_eq!(
                verdict(&manifest(), &runs),
                Verdict::Fail(Lane::ALL.map(|lane| not_neutral(lane, path, name)).to_vec()),
                "{path} {key} {shown:?}"
            );
        }
    }
}

#[test]
fn every_lane_shows_the_neutral_cgroup_path_exactly() {
    for shown in [Some("/workload/plasmosome-cell-7"), None] {
        let mut runs = runs();
        for lane in Lane::ALL {
            for run in 0..2 {
                let cgroup = &mut observation(&mut runs, lane, run, "hint-cgroup").fields;
                cgroup.remove("path");
                match shown {
                    Some(path) => cgroup.insert("path".to_string(), json!(path)),
                    None => cgroup.insert("cgroup".to_string(), json!("/plasmosome/cell-7")),
                };
            }
        }
        let not_neutral = |lane| {
            let name = NeutralName::CgroupPath;
            finding("hint-cgroup", Reason::NotNeutral { lane, name })
        };
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::Fail(Lane::ALL.map(not_neutral).to_vec()),
            "{shown:?}"
        );
    }
}

#[test]
fn a_non_neutral_name_in_one_run_only_is_listed_beside_its_noise() {
    let mut runs = runs();
    let fields = &mut observation(&mut runs, Lane::Absent, 1, "hint-mounts").fields;
    fields["/opt"]["options"] = json!("rw");
    let noise = Reason::Noise {
        lane: Lane::Absent,
        field: "/opt".to_string(),
    };
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![
            not_neutral(Lane::Absent, "/opt", MountKey::Options),
            finding("hint-mounts", noise),
            differs("hint-mounts", "/opt"),
        ])
    );
}

#[test]
fn non_neutral_names_are_listed_in_the_manifests_order_whichever_run_showed_them() {
    for swapped in [false, true] {
        let mut runs = runs();
        for (run, path) in [(0, "/usr/local/bin"), (1, "/opt")] {
            let mounts = &mut observation(&mut runs, Lane::Cell, run, "hint-mounts").fields;
            mounts[path]["source"] = json!("plasmosome");
        }
        if swapped {
            runs.swap(0, 1);
        }
        let noise = |field: &str| {
            let field = field.to_string();
            finding(
                "hint-mounts",
                Reason::Noise {
                    lane: Lane::Cell,
                    field,
                },
            )
        };
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::Fail(vec![
                not_neutral(Lane::Cell, "/opt", MountKey::Source),
                not_neutral(Lane::Cell, "/usr/local/bin", MountKey::Source),
                noise("/opt"),
                noise("/usr/local/bin"),
                differs("hint-mounts", "/opt"),
                differs("hint-mounts", "/usr/local/bin"),
            ]),
            "cell runs swapped: {swapped}"
        );
    }
}

#[test]
fn names_in_an_observation_that_did_not_complete_are_not_checked() {
    for failed_before_step in [false, true] {
        let mut runs = runs();
        let crashed = observation(&mut runs, Lane::Cell, 1, "hint-mounts");
        crashed.completed = failed_before_step;
        crashed.failed_before_step = failed_before_step;
        crashed.fields["/opt"]["source"] = json!("plasmosome");
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::NotRun {
                not_run: vec![finding("hint-mounts", Reason::NotRun { lane: Lane::Cell })],
                failures: vec![],
            },
            "failed before its step: {failed_before_step}"
        );
    }
}

#[test]
fn a_missing_mount_or_a_non_neutral_cgroup_path_is_not_neutral() {
    let mut runs = runs();
    for run in 0..2 {
        let mounts = &mut observation(&mut runs, Lane::Open, run, "hint-mounts").fields;
        mounts.remove("/opt");
        let cgroup = &mut observation(&mut runs, Lane::Open, run, "hint-cgroup").fields;
        cgroup.insert("path".to_string(), json!("/plasmosome/cell-7"));
    }
    let cgroup = Reason::NotNeutral {
        lane: Lane::Open,
        name: NeutralName::CgroupPath,
    };
    assert_eq!(
        verdict(&manifest(), &runs),
        Verdict::Fail(vec![
            not_neutral(Lane::Open, "/opt", MountKey::Source),
            not_neutral(Lane::Open, "/opt", MountKey::Fstype),
            not_neutral(Lane::Open, "/opt", MountKey::Options),
            finding("hint-cgroup", cgroup),
        ])
    );
}
