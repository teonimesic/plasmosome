use serde_json::{Value, json};

use super::{load_value, manifest, manifest_json};
use crate::differential::{Channel, ManifestRefusal, ReachKind, Rule, RuleKind, load};

fn probe_index(manifest: &Value, id: &str) -> usize {
    manifest["probes"]
        .as_array()
        .expect("probes is a list")
        .iter()
        .position(|probe| probe["id"] == id)
        .expect("the probe is declared")
}

fn without_probe(id: &str) -> Value {
    let mut manifest = manifest_json();
    let index = probe_index(&manifest, id);
    manifest["probes"]
        .as_array_mut()
        .expect("probes is a list")
        .remove(index);
    manifest
}

fn push(manifest: &mut Value, key: &str, entry: Value) {
    manifest[key].as_array_mut().expect("a list").push(entry);
}

fn refusal(manifest: &Value) -> ManifestRefusal {
    load_value(manifest).expect_err("the manifest is refused")
}

#[test]
fn a_channel_one_probe_short_is_refused_naming_it() {
    assert_eq!(
        refusal(&without_probe("op-times")),
        ManifestRefusal::ChannelShort {
            channel: Channel::Operation,
            count: 2
        }
    );
}

#[test]
fn a_kind_one_reach_probe_short_is_refused_naming_it() {
    let mut manifest = manifest_json();
    let index = probe_index(&manifest, "refusal-host-2");
    manifest["probes"][index]["reach"] = Value::Null;
    assert_eq!(
        refusal(&manifest),
        ManifestRefusal::KindShort {
            kind: ReachKind::Host,
            count: 2
        }
    );
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
fn only_the_systems_times_and_durations_keep_order() {
    let ordered: Vec<RuleKind> = RuleKind::ALL
        .iter()
        .copied()
        .filter(|kind| kind.keeps_order())
        .collect();
    assert_eq!(ordered, [RuleKind::SystemTime, RuleKind::Duration]);
}

#[test]
fn a_rule_on_a_field_the_workload_set_is_refused_naming_it() {
    let mut manifest = manifest_json();
    push(
        &mut manifest,
        "rules",
        json!({"probe": "op-utimensat", "field": "mtime", "kind": "system_time"}),
    );
    assert_eq!(
        refusal(&manifest),
        ManifestRefusal::RuleOnWorkloadSetField {
            probe: "op-utimensat".to_string(),
            field: "mtime".to_string()
        }
    );
}

#[test]
fn a_duplicate_probe_is_refused_naming_it() {
    let mut manifest = manifest_json();
    push(
        &mut manifest,
        "probes",
        json!({"id": "hint-env", "channel": "hint", "step": "before attach"}),
    );
    assert_eq!(
        refusal(&manifest),
        ManifestRefusal::DuplicateProbe {
            id: "hint-env".to_string()
        }
    );
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
        (rule, "rule"),
        (allowance, "allowance"),
        (mounts, "neutral"),
        (cgroup, "neutral"),
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
fn neutral_names_without_opt_usr_local_bin_or_a_fuse_workspace_are_refused() {
    for (index, mount) in [(0, "/opt"), (1, "/usr/local/bin"), (2, "a FUSE mount")] {
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
fn a_misspelt_key_or_text_that_is_not_json_is_refused_as_malformed() {
    let mut manifest = manifest_json();
    let top = manifest.as_object_mut().expect("an object");
    let rules = top.remove("rules").expect("the fixture has rules");
    top.insert("rule".to_string(), rules);
    for text in [manifest.to_string(), "not json".to_string()] {
        assert!(
            matches!(load(&text), Err(ManifestRefusal::Malformed { .. })),
            "{text}"
        );
    }
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
