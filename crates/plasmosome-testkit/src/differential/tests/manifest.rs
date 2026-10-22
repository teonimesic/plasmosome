use serde_json::{Value, json};

use super::{load_value, manifest, manifest_json};
use crate::differential::{
    Channel, ManifestRefusal, NamedBy, ReachKind, RequiredMount, Rule, RuleKind, load,
};

fn probe_index(manifest: &Value, id: &str) -> usize {
    manifest["probes"]
        .as_array()
        .expect("probes is a list")
        .iter()
        .position(|probe| probe["id"] == id)
        .expect("the probe is declared")
}

fn push(manifest: &mut Value, key: &str, entry: Value) {
    manifest[key].as_array_mut().expect("a list").push(entry);
}

fn refusal(manifest: &Value) -> ManifestRefusal {
    load_value(manifest).expect_err("the manifest is refused")
}

fn malformed(text: &str) -> String {
    match load(text) {
        Err(ManifestRefusal::Malformed { reason }) => reason,
        other => panic!("not refused as malformed: {other:?}"),
    }
}

const CHANNELS: [(Channel, &str); 5] = [
    (Channel::Refusal, "refusal"),
    (Channel::Hint, "hint"),
    (Channel::Operation, "operation"),
    (Channel::Facility, "facility"),
    (Channel::Disturbance, "disturbance"),
];

const KINDS: [(ReachKind, &str); 3] = [
    (ReachKind::Host, "host"),
    (ReachKind::Path, "path"),
    (ReachKind::Facility, "facility"),
];

#[test]
fn every_channel_left_with_no_probes_or_two_is_refused_naming_it() {
    assert_eq!(CHANNELS.map(|(channel, _)| channel), Channel::ALL);
    for (index, (channel, name)) in CHANNELS.into_iter().enumerate() {
        let elsewhere = CHANNELS[(index + 1) % CHANNELS.len()].1;
        for count in [0, 2] {
            let mut manifest = manifest_json();
            let mut kept = 0;
            for probe in manifest["probes"].as_array_mut().expect("probes is a list") {
                if probe["channel"] == name {
                    kept += 1;
                    if kept > count {
                        probe["channel"] = json!(elsewhere);
                    }
                }
            }
            assert_eq!(
                refusal(&manifest),
                ManifestRefusal::ChannelShort { channel, count },
                "{name} left with {count}"
            );
        }
    }
}

#[test]
fn every_kind_left_with_no_reach_probes_or_two_is_refused_naming_it() {
    assert_eq!(KINDS.map(|(kind, _)| kind), ReachKind::ALL);
    for (kind, name) in KINDS {
        for count in [0, 2] {
            let mut manifest = manifest_json();
            let mut kept = 0;
            for probe in manifest["probes"].as_array_mut().expect("probes is a list") {
                if probe["reach"] == name {
                    kept += 1;
                    if kept > count {
                        probe["reach"] = Value::Null;
                    }
                }
            }
            assert_eq!(
                refusal(&manifest),
                ManifestRefusal::KindShort { kind, count },
                "{name} left with {count}"
            );
        }
    }
}

#[test]
fn a_rule_kind_spec_027_does_not_list_is_refused_naming_it() {
    let mut manifest = manifest_json();
    push(
        &mut manifest,
        "rules",
        json!({"probe": "hint-env", "field": "TOKEN", "kind": "hex_token"}),
    );
    assert_eq!(
        refusal(&manifest),
        ManifestRefusal::UnknownRuleKind {
            probe: "hint-env".to_string(),
            field: "TOKEN".to_string(),
            kind: "hex_token".to_string()
        }
    );
}

#[test]
fn a_rule_kind_written_in_another_case_is_refused_naming_it() {
    for kind in ["PID", "System_Time", " pid", "pid "] {
        let mut manifest = manifest_json();
        push(
            &mut manifest,
            "rules",
            json!({"probe": "hint-env", "field": "TOKEN", "kind": kind}),
        );
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::UnknownRuleKind {
                probe: "hint-env".to_string(),
                field: "TOKEN".to_string(),
                kind: kind.to_string()
            }
        );
    }
}

#[test]
fn every_rule_kind_spec_027_lists_loads_by_its_name() {
    let mut manifest = manifest_json();
    for (index, kind) in RuleKind::ALL.iter().enumerate() {
        push(
            &mut manifest,
            "rules",
            json!({"probe": "hint-env", "field": format!("field-{index}"), "kind": kind.name()}),
        );
    }
    let loaded = load_value(&manifest).expect("every listed kind loads");
    let kinds: Vec<RuleKind> = loaded.rules.iter().skip(7).map(|rule| rule.kind).collect();
    assert_eq!(kinds, RuleKind::ALL.to_vec());
    let names: Vec<&str> = RuleKind::ALL.iter().map(|kind| kind.name()).collect();
    assert_eq!(
        names,
        [
            "pid",
            "boot_id",
            "mount_id",
            "parent_mount_id",
            "peer_group",
            "device",
            "inode",
            "system_time",
            "duration",
            "ephemeral_port",
            "resolved_address",
            "memory_counter",
            "load_counter"
        ]
    );
}

#[test]
fn the_peer_group_kind_loads_by_its_exact_name() {
    let rule = |kind: &str| json!({"probe": "hint-env", "field": "masters", "kind": kind});
    let mut manifest = manifest_json();
    push(&mut manifest, "rules", rule("peer_group"));
    let loaded = load_value(&manifest).expect("peer_group loads");
    assert_eq!(
        loaded.rules.last(),
        Some(&Rule {
            probe: "hint-env".to_string(),
            field: "masters".to_string(),
            kind: RuleKind::PeerGroup
        })
    );
    for kind in ["Peer_Group", "peergroup", "peer_group_id"] {
        let mut manifest = manifest_json();
        push(&mut manifest, "rules", rule(kind));
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::UnknownRuleKind {
                probe: "hint-env".to_string(),
                field: "masters".to_string(),
                kind: kind.to_string()
            },
            "{kind}"
        );
    }
}

#[test]
fn a_rule_of_any_kind_on_a_field_the_workload_set_is_refused_naming_it() {
    for kind in RuleKind::ALL {
        let mut manifest = manifest_json();
        push(
            &mut manifest,
            "rules",
            json!({"probe": "op-utimensat", "field": "mtime", "kind": kind.name()}),
        );
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::RuleOnWorkloadSetField {
                probe: "op-utimensat".to_string(),
                field: "mtime".to_string()
            },
            "{}",
            kind.name()
        );
    }
}

#[test]
fn a_rule_on_a_probes_second_workload_set_field_is_refused_naming_it() {
    let mut manifest = manifest_json();
    let index = probe_index(&manifest, "op-utimensat");
    manifest["probes"][index]["workload_set"] = json!(["mtime", "atime"]);
    push(
        &mut manifest,
        "rules",
        json!({"probe": "op-utimensat", "field": "atime", "kind": "system_time"}),
    );
    assert_eq!(
        refusal(&manifest),
        ManifestRefusal::RuleOnWorkloadSetField {
            probe: "op-utimensat".to_string(),
            field: "atime".to_string()
        }
    );
}

#[test]
fn a_field_the_workload_set_in_one_probe_may_have_a_rule_in_another() {
    let mut manifest = manifest_json();
    push(
        &mut manifest,
        "rules",
        json!({"probe": "op-times", "field": "mtime", "kind": "system_time"}),
    );
    assert!(load_value(&manifest).is_ok());
}

#[test]
fn a_probe_that_leaves_out_its_workload_set_or_gives_null_is_refused_naming_it() {
    for (id, null) in [
        ("op-times", false),
        ("op-times", true),
        ("facility-2", false),
        ("facility-2", true),
    ] {
        let mut manifest = manifest_json();
        let index = probe_index(&manifest, id);
        let probe = manifest["probes"][index]
            .as_object_mut()
            .expect("a probe is an object");
        probe.remove("workload_set");
        if null {
            probe.insert("workload_set".to_string(), Value::Null);
        }
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::WorkloadSetMissing {
                probe: id.to_string()
            },
            "{id} null: {null}"
        );
    }
}

#[test]
fn a_probe_that_leaves_out_its_step_is_refused() {
    let mut manifest = manifest_json();
    let index = probe_index(&manifest, "op-times");
    manifest["probes"][index]
        .as_object_mut()
        .expect("a probe is an object")
        .remove("step");
    let reason = malformed(&manifest.to_string());
    assert!(reason.contains("missing field `step`"), "{reason}");
}

#[test]
fn a_key_repeated_in_one_object_is_refused_naming_it() {
    let text = manifest_json().to_string();
    let repeated = |once: &str, twice: &str| {
        assert_eq!(text.matches(once).count(), 1, "{once}");
        text.replacen(once, twice, 1)
    };
    for (manifest, key) in [
        (
            repeated(r#""probes":["#, r#""rules":[],"probes":["#),
            "rules",
        ),
        (
            repeated(
                r#""workload_set":["mtime"]"#,
                r#""workload_set":["mtime"],"workload_set":[]"#,
            ),
            "workload_set",
        ),
        (
            repeated(
                r#""cell_value":"0600""#,
                r#""cell_value":"0600","cell_value":"0777""#,
            ),
            "cell_value",
        ),
    ] {
        let reason = malformed(&manifest);
        assert!(
            reason.contains(&format!("duplicate field `{key}`")),
            "{reason}"
        );
    }
}

#[test]
fn a_key_repeated_inside_an_allowed_value_is_refused_at_any_depth_naming_it() {
    for (cell_value, path) in [
        (json!({"mode": "0600"}), "/mode"),
        (json!({"stat": {"mode": "0600"}}), "/stat/mode"),
        (json!([{"mode": "0600"}]), "/0/mode"),
        (
            json!({"stat": [1, {"xattr": {"mode": "0600"}}]}),
            "/stat/1/xattr/mode",
        ),
    ] {
        let mut manifest = manifest_json();
        manifest["allowances"][0]["cell_value"] = cell_value;
        let text = manifest.to_string();
        let once = r#""mode":"0600""#;
        assert_eq!(text.matches(once).count(), 1, "{path}");
        let reason = malformed(&text.replacen(once, r#""mode":"0600","mode":"0777""#, 1));
        assert!(
            reason.contains(&format!("duplicate object key at \"{path}\"")),
            "{path}: {reason}"
        );
    }
}

#[test]
fn an_allowed_value_that_is_not_json_is_refused_as_malformed() {
    let text = manifest_json().to_string();
    let once = r#""cell_value":"0600""#;
    assert_eq!(text.matches(once).count(), 1);
    let reason = malformed(&text.replacen(once, r#""cell_value":}"#, 1));
    assert!(reason.contains("expected value"), "{reason}");
}

#[test]
fn an_allowed_value_of_any_json_type_loads_unchanged() {
    let value = json!({
        "stat": {"mode": "0600", "uid": 1000, "offset": -1, "ratio": 0.5, "size": u64::MAX},
        "flags": [true, false, null, [], {}],
        "name": "notes.txt"
    });
    let mut manifest = manifest_json();
    manifest["allowances"][0]["cell_value"] = value.clone();
    let loaded = load_value(&manifest).expect("the manifest loads");
    assert_eq!(loaded.allowances[0].cell_value, value);
}

#[test]
fn a_probe_id_used_twice_is_refused_in_the_same_channel_or_another() {
    let same =
        json!({"id": "hint-env", "channel": "hint", "step": "before attach", "workload_set": []});
    let other = json!({"id": "op-times", "channel": "refusal", "step": "after attach", "reach": "host", "workload_set": []});
    for (id, again) in [("hint-env", same), ("op-times", other)] {
        let mut manifest = manifest_json();
        push(&mut manifest, "probes", again);
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::DuplicateProbe { id: id.to_string() }
        );
    }
}

#[test]
fn a_rule_an_allowance_or_a_neutral_name_for_an_unknown_probe_is_refused_naming_it() {
    let mut rule = manifest_json();
    push(
        &mut rule,
        "rules",
        json!({"probe": "ghost", "field": "pid", "kind": "pid"}),
    );
    let mut allowance = manifest_json();
    push(
        &mut allowance,
        "allowances",
        json!({"probe": "ghost", "field": "mode", "cell_value": "0600"}),
    );
    let mut mounts = manifest_json();
    mounts["neutral"]["mounts_probe"] = json!("ghost");
    let mut cgroup = manifest_json();
    cgroup["neutral"]["cgroup_probe"] = json!("ghost");
    for (manifest, named_by) in [
        (rule, NamedBy::Rule),
        (allowance, NamedBy::Allowance),
        (mounts, NamedBy::MountsProbe),
        (cgroup, NamedBy::CgroupProbe),
    ] {
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::UnknownProbe {
                probe: "ghost".to_string(),
                named_by
            }
        );
    }
}

#[test]
fn two_rules_or_two_allowances_on_one_field_are_refused_naming_it() {
    let mut rules = manifest_json();
    push(
        &mut rules,
        "rules",
        json!({"probe": "op-times", "field": "times", "kind": "duration"}),
    );
    assert_eq!(
        refusal(&rules),
        ManifestRefusal::DuplicateRule {
            probe: "op-times".to_string(),
            field: "times".to_string()
        }
    );
    let mut allowances = manifest_json();
    push(
        &mut allowances,
        "allowances",
        json!({"probe": "op-utimensat", "field": "mode", "cell_value": "0640"}),
    );
    assert_eq!(
        refusal(&allowances),
        ManifestRefusal::DuplicateAllowance {
            probe: "op-utimensat".to_string(),
            field: "mode".to_string()
        }
    );
}

#[test]
fn a_rule_and_an_allowance_on_one_field_are_refused_naming_it() {
    let mut manifest = manifest_json();
    push(
        &mut manifest,
        "allowances",
        json!({"probe": "op-times", "field": "times", "cell_value": [1, 2]}),
    );
    assert_eq!(
        refusal(&manifest),
        ManifestRefusal::RuleAndAllowance {
            probe: "op-times".to_string(),
            field: "times".to_string()
        }
    );
}

#[test]
fn an_allowance_may_name_a_field_another_probe_has_a_rule_on() {
    let mut manifest = manifest_json();
    push(
        &mut manifest,
        "allowances",
        json!({"probe": "hint-self", "field": "pids", "cell_value": [1]}),
    );
    assert!(load_value(&manifest).is_ok());
}

#[test]
fn neutral_names_without_opt_or_usr_local_bin_are_refused() {
    for (index, mount) in [(0, RequiredMount::Opt), (1, RequiredMount::UsrLocalBin)] {
        let mut manifest = manifest_json();
        manifest["neutral"]["mounts"]
            .as_array_mut()
            .expect("mounts is a list")
            .remove(index);
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::NeutralMountMissing { mount }
        );
    }
}

#[test]
fn neutral_names_need_a_fuse_mount_at_a_path_of_its_own() {
    for fuse_at in [None, Some(0), Some(1)] {
        let mut manifest = manifest_json();
        let mounts = &mut manifest["neutral"]["mounts"];
        mounts[2]["fstype"] = json!("virtiofs");
        if let Some(index) = fuse_at {
            mounts[index]["fstype"] = json!("fuse");
        }
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::NeutralMountMissing {
                mount: RequiredMount::FuseWorkspace
            },
            "FUSE at {fuse_at:?}"
        );
    }
}

#[test]
fn only_the_type_fuse_itself_makes_a_workspace() {
    for fstype in ["fuseblk", "fuse.passthrough"] {
        let mut manifest = manifest_json();
        manifest["neutral"]["mounts"][2]["fstype"] = json!(fstype);
        assert_eq!(
            refusal(&manifest),
            ManifestRefusal::NeutralMountMissing {
                mount: RequiredMount::FuseWorkspace
            },
            "{fstype}"
        );
    }
}

#[test]
fn two_neutral_mounts_at_one_path_are_refused_naming_it() {
    let mut manifest = manifest_json();
    let again =
        json!({"path": "/opt", "source": "software", "fstype": "virtiofs", "options": "rw"});
    manifest["neutral"]["mounts"]
        .as_array_mut()
        .expect("mounts is a list")
        .push(again);
    assert_eq!(
        refusal(&manifest),
        ManifestRefusal::DuplicateNeutralMount {
            path: "/opt".to_string()
        }
    );
}

#[test]
fn an_unknown_key_at_any_level_is_refused_naming_it() {
    let mut top = manifest_json();
    let object = top.as_object_mut().expect("an object");
    let rules = object.remove("rules").expect("the fixture has rules");
    object.insert("rule".to_string(), rules);
    let mut probe = manifest_json();
    probe["probes"][0]["reachh"] = json!("host");
    let mut rule = manifest_json();
    rule["rules"][0]["kindd"] = json!("pid");
    let mut allowance = manifest_json();
    allowance["allowances"][0]["cell_valu"] = json!("0600");
    let mut neutral = manifest_json();
    neutral["neutral"]["fuse_subtypes"] = json!("passthrough");
    let mut mount = manifest_json();
    mount["neutral"]["mounts"][2]["subtype"] = json!("passthrough");
    for (manifest, key) in [
        (top, "rule"),
        (probe, "reachh"),
        (rule, "kindd"),
        (allowance, "cell_valu"),
        (neutral, "fuse_subtypes"),
        (mount, "subtype"),
    ] {
        let reason = malformed(&manifest.to_string());
        assert!(
            reason.contains(&format!("unknown field `{key}`")),
            "{reason}"
        );
    }
}

#[test]
fn text_that_is_not_json_or_has_trailing_data_is_refused_as_malformed() {
    malformed("not json");
    let reason = malformed(&format!("{} {{}}", manifest_json()));
    assert!(reason.contains("trailing characters"), "{reason}");
}

#[test]
fn a_manifest_meeting_the_rules_loads_every_table() {
    let loaded = manifest();
    assert_eq!(loaded.probes.len(), 17);
    assert_eq!(loaded.probes[16].workload_set, ["mtime"]);
    assert_eq!(
        loaded.rules[5],
        Rule {
            probe: "op-times".to_string(),
            field: "times".to_string(),
            kind: RuleKind::SystemTime
        }
    );
    assert_eq!(loaded.allowances[0].cell_value, json!("0600"));
    assert_eq!(loaded.neutral.mounts[2].fstype, "fuse");
    assert_eq!(loaded.validate(), Ok(()));
}

#[test]
fn validate_refuses_a_hand_built_manifest_the_way_load_does() {
    let mut manifest = manifest();
    manifest.probes.retain(|probe| probe.id != "op-times");
    assert_eq!(
        manifest.validate(),
        Err(ManifestRefusal::ChannelShort {
            channel: Channel::Operation,
            count: 2
        })
    );
}
