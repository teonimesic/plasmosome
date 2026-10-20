mod common;

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use plasmosome_lanes::probe;
use serde_json::json;

#[test]
fn errno_names_are_symbolic() {
    assert_eq!(probe::errno_name(libc::ENOENT), "ENOENT");
    assert_eq!(probe::errno_name(libc::EACCES), "EACCES");
    assert_eq!(probe::errno_name(libc::EROFS), "EROFS");
    assert_eq!(probe::errno_name(libc::EPERM), "EPERM");
    assert_eq!(probe::errno_name(libc::EEXIST), "EEXIST");
    assert_eq!(probe::errno_name(libc::ENOTDIR), "ENOTDIR");
    assert_eq!(probe::errno_name(libc::ENOTEMPTY), "ENOTEMPTY");
    assert_eq!(probe::errno_name(libc::ESTALE), "ESTALE");
    assert_eq!(probe::errno_name(libc::EXDEV), "EXDEV");
}

#[test]
fn an_errno_with_no_name_is_reported_by_number() {
    assert_eq!(probe::errno_name(9999), "errno-9999");
}

#[test]
fn stat_reports_owner_mode_kind_and_size() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = common::write_file(dir.path(), "placed", b"twelve bytes");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("mode");
    let metadata = fs::metadata(&path).expect("metadata");

    let line = probe::stat(&path);

    assert_eq!(line["probe"], "stat");
    assert_eq!(line["ok"], true);
    assert_eq!(line["kind"], "file");
    assert_eq!(line["mode"], "0640");
    assert_eq!(line["size"], 12);
    assert_eq!(line["uid"], metadata.uid());
    assert_eq!(line["gid"], metadata.gid());
    assert_eq!(line["ino"], metadata.ino());
    assert_eq!(line["dev"], metadata.dev());
}

#[test]
fn stat_of_a_directory_reports_its_kind() {
    let dir = tempfile::tempdir().expect("a scratch directory");

    assert_eq!(probe::stat(dir.path())["kind"], "dir");
}

#[test]
fn stat_of_a_missing_path_names_enoent() {
    let dir = tempfile::tempdir().expect("a scratch directory");

    let line = probe::stat(&dir.path().join("missing"));

    assert_eq!(
        line,
        json!({"probe": "stat", "ok": false, "errno": "ENOENT"})
    );
}

#[test]
fn write_appends_to_an_existing_file() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = common::write_file(dir.path(), "placed", b"first\n");

    let line = probe::write(&path);

    assert_eq!(line["ok"], true, "{line}");
    let text = fs::read_to_string(&path).expect("read");
    assert!(
        text.starts_with("first\n") && text.len() > 6,
        "write appended: {text:?}"
    );
}

#[test]
fn write_does_not_create_a_missing_file() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("missing");

    assert_eq!(probe::write(&path)["errno"], "ENOENT");
    assert!(!path.exists());
}

#[test]
fn write_to_a_read_only_file_names_eacces() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = common::write_file(dir.path(), "placed", b"first\n");
    let uid = fs::metadata(&path).expect("metadata").uid();
    assert_ne!(uid, 0, "this test needs a non-root user");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).expect("mode");

    assert_eq!(probe::write(&path)["errno"], "EACCES");
}

#[test]
fn create_makes_a_new_file_and_refuses_an_existing_one() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("made");

    assert_eq!(probe::create(&path)["ok"], true);
    assert!(path.exists());
    assert_eq!(probe::create(&path)["errno"], "EEXIST");
}

#[test]
fn list_reports_sorted_names() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    common::write_file(dir.path(), "zeta", b"");
    common::write_file(dir.path(), "alpha", b"");
    fs::create_dir(dir.path().join("mid")).expect("dir");

    let line = probe::list(dir.path());

    assert_eq!(line["ok"], true);
    assert_eq!(line["names"], json!(["alpha", "mid", "zeta"]));
}

#[test]
fn list_of_an_empty_directory_is_an_empty_list_not_an_error() {
    let dir = tempfile::tempdir().expect("a scratch directory");

    assert_eq!(
        probe::list(dir.path()),
        json!({"probe": "list", "ok": true, "names": []})
    );
}

#[test]
fn list_of_a_missing_or_non_directory_path_names_its_errno() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let file = common::write_file(dir.path(), "placed", b"");

    assert_eq!(probe::list(&dir.path().join("missing"))["errno"], "ENOENT");
    assert_eq!(probe::list(&file)["errno"], "ENOTDIR");
}

#[test]
fn lookup_does_not_follow_a_final_symlink() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let link = dir.path().join("link");
    symlink(dir.path().join("nowhere"), &link).expect("symlink");

    let line = probe::lookup(&link);

    assert_eq!(line["ok"], true, "{line}");
    assert_eq!(line["kind"], "symlink");
    assert_eq!(probe::stat(&link)["errno"], "ENOENT");
}

#[test]
fn lookup_of_a_missing_name_names_enoent() {
    let dir = tempfile::tempdir().expect("a scratch directory");

    assert_eq!(
        probe::lookup(&dir.path().join("gone")),
        json!({"probe": "lookup", "ok": false, "errno": "ENOENT"})
    );
}

#[test]
fn id_reads_the_four_uids_and_gids_and_the_groups_from_a_status_text() {
    let status = "Name:\tlane-probe\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1001\t1000\t1000\nGroups:\t\nNoNewPrivs:\t0\n";

    assert_eq!(
        probe::id_from_status(status),
        json!({"probe": "id", "ok": true, "uid": 1000, "euid": 1000, "gid": 1000, "egid": 1001, "groups": []})
    );
}

#[test]
fn id_with_no_uid_line_is_a_failure_not_a_zero() {
    let line = probe::id_from_status("Name:\tlane-probe\n");

    assert_eq!(line["ok"], false, "{line}");
}

#[test]
fn filesystems_lists_every_name() {
    let text = "nodev\tsysfs\nnodev\tproc\n\text4\nnodev\tfuse\nnodev\tvirtiofs\n";

    assert_eq!(
        probe::filesystems_from(text),
        json!({"probe": "filesystems", "ok": true, "names": ["sysfs", "proc", "ext4", "fuse", "virtiofs"]})
    );
}

#[test]
fn mountinfo_reports_mount_point_type_source_and_options() {
    let text = "22 1 0:5 / /dev rw,relatime shared:2 - devtmpfs devtmpfs rw,size=1k\n\
                31 1 0:28 / /mnt/share ro,nodev,relatime - virtiofs lanes0 rw\n";

    assert_eq!(
        probe::mountinfo_from(text),
        json!({"probe": "mountinfo", "ok": true, "mounts": [
            {"mount_point": "/dev", "root": "/", "optional_fields": ["shared:2"], "fstype": "devtmpfs", "source": "devtmpfs", "mount_options": "rw,relatime", "super_options": "rw,size=1k"},
            {"mount_point": "/mnt/share", "root": "/", "optional_fields": [], "fstype": "virtiofs", "source": "lanes0", "mount_options": "ro,nodev,relatime", "super_options": "rw"}
        ]})
    );
}

#[test]
fn mountinfo_keeps_each_mounts_root_and_its_propagation_fields() {
    let text = "40 31 0:28 /membrane /opt rw,relatime shared:3 master:1 - virtiofs lanes0 rw\n\
                41 31 0:28 / /opt rw,relatime - virtiofs lanes0 rw\n";

    let line = probe::mountinfo_from(text);

    assert_eq!(line["mounts"][0]["root"], "/membrane");
    assert_eq!(
        line["mounts"][0]["optional_fields"],
        json!(["shared:3", "master:1"])
    );
    assert_ne!(
        line["mounts"][0], line["mounts"][1],
        "a bind of the membrane and a plain mount must not compare equal"
    );
}

#[test]
fn a_listing_with_a_name_that_is_not_utf8_is_refused_rather_than_merged() {
    let names = vec![
        OsString::from("alpha"),
        OsString::from_vec(vec![b'a', 0xff]),
        OsString::from_vec(vec![b'a', 0xfe]),
    ];

    let line = probe::list_names(names);

    assert_eq!(line["ok"], false, "{line}");
    let error = line["error"].as_str().expect("an error text");
    assert!(error.contains("UTF-8"), "{error}");
}

#[test]
fn listed_names_are_sorted() {
    let line = probe::list_names(vec![OsString::from("zeta"), OsString::from("alpha")]);

    assert_eq!(
        line,
        json!({"probe": "list", "ok": true, "names": ["alpha", "zeta"]})
    );
}

#[test]
fn a_holder_name_must_be_a_plain_name_inside_the_holder_directory() {
    for name in ["held-fd", "a.b", "A_1", "0"] {
        assert!(probe::holder_name_ok(name), "`{name}` should be accepted");
    }
    for name in ["", ".", "..", "../x", "/tmp/x", "a/b", ".hidden", "-x"] {
        assert!(!probe::holder_name_ok(name), "`{name}` should be refused");
    }
}

#[test]
fn a_failure_line_names_the_probe_and_the_symbolic_errno() {
    let error = std::io::Error::from_raw_os_error(libc::EROFS);

    assert_eq!(
        probe::failure("create", &error),
        json!({"probe": "create", "ok": false, "errno": "EROFS"})
    );
}

fn sysfs(root: &std::path::Path) {
    common::write_file(root, "bus/virtio/devices/virtio1/device", b"0x0002\n");
    common::write_file(root, "bus/virtio/devices/virtio0/device", b"0x0013\n");
    common::write_file(root, "block/vda/ro", b"0\n");
    common::write_file(root, "block/vdb/ro", b"1\n");
}

#[test]
fn devices_reports_each_virtio_device_id_and_whether_each_block_device_is_read_only() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    sysfs(dir.path());

    assert_eq!(
        probe::devices(dir.path()),
        json!({"probe": "devices", "ok": true,
               "virtio": [{"name": "virtio0", "device": "0x0013"}, {"name": "virtio1", "device": "0x0002"}],
               "block": [{"name": "vda", "read_only": false}, {"name": "vdb", "read_only": true}]})
    );
}

#[test]
fn devices_without_sysfs_names_enoent() {
    let dir = tempfile::tempdir().expect("a scratch directory");

    let line = probe::devices(&dir.path().join("missing"));

    assert_eq!(line["ok"], false, "{line}");
    assert_eq!(line["errno"], "ENOENT", "{line}");
}

#[test]
fn a_read_only_flag_that_is_neither_0_nor_1_fails_the_probe() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    sysfs(dir.path());
    common::write_file(dir.path(), "block/vda/ro", b"maybe\n");

    let line = probe::devices(dir.path());

    assert_eq!(line["ok"], false, "{line}");
    assert!(
        line["error"]
            .as_str()
            .is_some_and(|error| error.contains("vda")),
        "{line}"
    );
}
