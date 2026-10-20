mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use plasmosome_lanes::config::{GuestConfig, LaneConfig, Semantics};
use serde_json::{Value, json};

fn lane(value: &Value) -> Result<LaneConfig, String> {
    LaneConfig::from_json(&value.to_string()).map_err(|error| error.to_string())
}

fn guest(value: &Value) -> Result<GuestConfig, String> {
    GuestConfig::from_json(&value.to_string()).map_err(|error| error.to_string())
}

fn refused(result: Result<impl std::fmt::Debug, String>, naming: &str) {
    match result {
        Ok(parsed) => panic!("expected a refusal naming `{naming}`, it parsed as {parsed:?}"),
        Err(message) => assert!(
            message.contains(naming),
            "the refusal should name `{naming}`; it said: {message}"
        ),
    }
}

#[test]
fn a_complete_lane_configuration_parses_to_its_fields() {
    let root = Path::new("/lanes");
    let parsed = lane(&common::lane_json(root)).expect("the fixture configuration is valid");

    assert_eq!(parsed, common::lane_config(root, "run-1"));
}

#[test]
fn a_lane_configuration_round_trips_through_its_json() {
    let config = common::lane_config(Path::new("/lanes"), "run-1");

    let json = config.to_json().expect("the configuration serializes");
    assert_eq!(lane(&json.parse().expect("JSON")), Ok(config));
}

#[test]
fn an_unknown_top_level_field_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["vsock_port"] = json!(4090);

    refused(lane(&value), "vsock_port");
}

#[test]
fn an_unknown_share_field_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["shares"][0]["cache"] = json!("always");

    refused(lane(&value), "cache");
}

#[test]
fn a_missing_field_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value.as_object_mut().expect("an object").remove("console");

    refused(lane(&value), "console");
}

#[test]
fn a_missing_confinement_is_refused_rather_than_read_as_none() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value
        .as_object_mut()
        .expect("an object")
        .remove("confinement");

    refused(lane(&value), "confinement");
}

#[test]
fn a_share_whose_path_is_relative_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["shares"][0]["path"] = json!("share/relative");

    refused(lane(&value), "share/relative");
}

#[test]
fn a_relative_bundle_path_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["bundle"] = json!("bundles/eed141b574337d99");

    refused(lane(&value), "bundles/eed141b574337d99");
}

#[test]
fn a_relative_confinement_profile_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["confinement"] =
        json!({"profile": "lane.sb", "allowed": "/lanes/allowed", "outside": "/lanes/outside"});

    refused(lane(&value), "lane.sb");
}

#[test]
fn a_duplicate_share_tag_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    let share = value["shares"][0].clone();
    let mut second = share.clone();
    second["path"] = json!("/lanes/other");
    value["shares"] = json!([share, second]);

    refused(lane(&value), "lanes0");
}

#[test]
fn a_semantics_value_other_than_complete_or_simplified_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["shares"][0]["semantics"] = json!("relaxed");

    refused(lane(&value), "relaxed");
}

#[test]
fn simplified_semantics_parses() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["shares"][0]["semantics"] = json!("simplified");

    let parsed = lane(&value).expect("simplified is a known value");
    assert_eq!(parsed.shares[0].semantics, Semantics::Simplified);
}

#[test]
fn semantics_map_to_the_pinned_header_values() {
    assert_eq!(Semantics::Complete.krun_value(), 0);
    assert_eq!(Semantics::Simplified.krun_value(), 1);
}

#[test]
fn zero_vcpus_are_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["vcpus"] = json!(0);

    refused(lane(&value), "vcpus");
}

#[test]
fn a_run_id_with_a_space_is_refused_because_it_shares_a_marker_line() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["run_id"] = json!("run 1");

    refused(lane(&value), "run_id");
}

#[test]
fn a_share_tag_longer_than_virtio_fs_allows_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    let long = "t".repeat(37);
    value["shares"][0]["tag"] = json!(long);

    refused(lane(&value), &long);
}

#[test]
fn a_guest_configuration_parses_and_a_probe_uid_defaults_to_1000() {
    let parsed = guest(&common::guest_json()).expect("the fixture guest configuration is valid");

    assert_eq!(parsed.probes[0].uid, 1000);
    assert_eq!(parsed.probes[1].uid, 0);
    assert_eq!(parsed.probe_ids(), vec!["uname", "root-id"]);
}

#[test]
fn a_guest_configuration_round_trips_through_its_json() {
    let config = common::guest_config("run-1", &["a", "b"]);

    let json = config.to_json().expect("the configuration serializes");
    assert_eq!(guest(&json.parse().expect("JSON")), Ok(config));
}

#[test]
fn an_unknown_guest_field_is_refused() {
    let mut value = common::guest_json();
    value["probes"][0]["env"] = json!({"PATH": "/bin"});

    refused(guest(&value), "env");
}

#[test]
fn a_missing_guest_field_is_refused() {
    let mut value = common::guest_json();
    value.as_object_mut().expect("an object").remove("mounts");

    refused(guest(&value), "mounts");
}

#[test]
fn a_relative_guest_mount_path_is_refused() {
    let mut value = common::guest_json();
    value["mounts"][0]["path"] = json!("mnt/share");

    refused(guest(&value), "mnt/share");
}

#[test]
fn an_unknown_mount_option_is_refused() {
    let mut value = common::guest_json();
    value["mounts"][0]["options"] = json!(["ro", "dax"]);

    refused(guest(&value), "dax");
}

#[test]
fn a_duplicate_guest_mount_tag_is_refused() {
    let mut value = common::guest_json();
    let mount = value["mounts"][0].clone();
    let mut second = mount.clone();
    second["path"] = json!("/mnt/other");
    value["mounts"] = json!([mount, second]);

    refused(guest(&value), "lanes0");
}

#[test]
fn a_duplicate_probe_id_is_refused() {
    let mut value = common::guest_json();
    value["probes"][1]["id"] = json!("uname");

    refused(guest(&value), "uname");
}

#[test]
fn a_probe_with_no_arguments_is_refused() {
    let mut value = common::guest_json();
    value["probes"][0]["args"] = json!([]);

    refused(guest(&value), "uname");
}

#[test]
fn a_guest_run_id_with_a_space_is_refused() {
    let mut value = common::guest_json();
    value["run_id"] = json!("run 1");

    refused(guest(&value), "run_id");
}

fn with_nul() -> String {
    format!("/lanes/a{}b", char::from(0))
}

fn not_utf8() -> PathBuf {
    PathBuf::from(OsStr::from_bytes(&[b'/', b'l', 0xff]))
}

#[test]
fn a_relative_root_image_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["root_image"] = json!("root.ext4");

    refused(lane(&value), "root_image");
}

#[test]
fn a_relative_initramfs_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["initramfs"] = json!("initramfs.cpio");

    refused(lane(&value), "initramfs");
}

#[test]
fn a_relative_console_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["console"] = json!("console.log");

    refused(lane(&value), "console");
}

#[test]
fn zero_memory_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["memory_mib"] = json!(0);

    refused(lane(&value), "memory_mib");
}

#[test]
fn a_share_tag_of_exactly_36_bytes_parses() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["shares"][0]["tag"] = json!("t".repeat(36));

    let parsed = lane(&value).expect("36 bytes is the virtio-fs limit, not past it");
    assert_eq!(parsed.shares[0].tag.len(), 36);
}

#[test]
fn an_empty_share_tag_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["shares"][0]["tag"] = json!("");

    refused(lane(&value), "tag");
}

#[test]
fn a_share_tag_with_a_nul_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["shares"][0]["tag"] = json!(format!("lanes{}0", char::from(0)));

    refused(lane(&value), "NUL");
}

#[test]
fn a_relative_confinement_allowed_directory_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["confinement"] =
        json!({"profile": "/lanes/lane.sb", "allowed": "allowed", "outside": "/lanes/outside"});

    refused(lane(&value), "confinement.allowed");
}

#[test]
fn a_relative_confinement_outside_directory_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["confinement"] =
        json!({"profile": "/lanes/lane.sb", "allowed": "/lanes/allowed", "outside": "outside"});

    refused(lane(&value), "confinement.outside");
}

#[test]
fn a_complete_confinement_parses() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["confinement"] = json!({
        "profile": "/lanes/lane.sb", "allowed": "/lanes/allowed", "outside": "/lanes/outside"
    });

    let parsed = lane(&value).expect("every confinement path is absolute");
    let confinement = parsed.confinement.expect("a confinement");
    assert_eq!(confinement.outside, Path::new("/lanes/outside"));
}

#[test]
fn a_path_with_a_nul_byte_is_refused() {
    let mut value = common::lane_json(Path::new("/lanes"));
    value["bundle"] = json!(with_nul());

    refused(lane(&value), "NUL");
}

#[test]
fn a_guest_mount_tag_longer_than_virtio_fs_allows_is_refused() {
    let mut value = common::guest_json();
    let long = "t".repeat(37);
    value["mounts"][0]["tag"] = json!(long);

    refused(guest(&value), &long);
}

#[test]
fn an_empty_guest_mount_tag_is_refused() {
    let mut value = common::guest_json();
    value["mounts"][0]["tag"] = json!("");

    refused(guest(&value), "tag");
}

#[test]
fn a_guest_mount_path_with_a_nul_byte_is_refused() {
    let mut value = common::guest_json();
    value["mounts"][0]["path"] = json!(with_nul());

    refused(guest(&value), "NUL");
}

#[test]
fn an_empty_probe_id_is_refused() {
    let mut value = common::guest_json();
    value["probes"][0]["id"] = json!("");

    refused(guest(&value), "probe id");
}

#[test]
fn a_lane_path_that_is_not_utf8_refuses_to_serialize_naming_its_field() {
    let mut config = common::lane_config(Path::new("/lanes"), "run-1");
    config.shares[0].path = not_utf8();

    let error = config.to_json().expect_err("a path that is not UTF-8");
    assert!(error.to_string().contains("share"), "{error}");
}

#[test]
fn a_guest_mount_path_that_is_not_utf8_refuses_to_serialize_naming_its_field() {
    let mut config = common::guest_config("run-1", &["id"]);
    config.mounts[0].path = not_utf8();

    let error = config.to_json().expect_err("a path that is not UTF-8");
    assert!(error.to_string().contains("mount"), "{error}");
}
