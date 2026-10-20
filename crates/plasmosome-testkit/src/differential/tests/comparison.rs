use serde_json::{Value, json};

use super::{differs, finding, manifest, observation, runs, set, verdict};
use crate::differential::{Finding, Lane, MountKey, NeutralName, Reason, Verdict};

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
fn every_neutral_mount_name_is_checked_in_every_lane() {
    for (path, key, name) in [
        ("/usr/local/bin", "fstype", MountKey::Fstype),
        ("/opt", "options", MountKey::Options),
        ("/workspace", "subtype", MountKey::Subtype),
        ("/workspace", "source", MountKey::Source),
    ] {
        let mut runs = runs();
        for lane in Lane::ALL {
            for run in 0..2 {
                let fields = &mut observation(&mut runs, lane, run, "hint-mounts").fields;
                fields[path][key] = json!("plasmosome");
            }
        }
        assert_eq!(
            verdict(&manifest(), &runs),
            Verdict::Fail(Lane::ALL.map(|lane| not_neutral(lane, path, name)).to_vec()),
            "{path} {key}"
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
