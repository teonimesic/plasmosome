mod common;

use std::fs;
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
            {"mount_point": "/dev", "fstype": "devtmpfs", "source": "devtmpfs", "mount_options": "rw,relatime", "super_options": "rw,size=1k"},
            {"mount_point": "/mnt/share", "fstype": "virtiofs", "source": "lanes0", "mount_options": "ro,nodev,relatime", "super_options": "rw"}
        ]})
    );
}

#[test]
fn a_failure_line_names_the_probe_and_the_symbolic_errno() {
    let error = std::io::Error::from_raw_os_error(libc::EROFS);

    assert_eq!(
        probe::failure("create", &error),
        json!({"probe": "create", "ok": false, "errno": "EROFS"})
    );
}
