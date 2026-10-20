mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;

use plasmosome_lanes::config::GuestConfig;
use plasmosome_lanes::cpio::{
    Entry, S_IFCHR, S_IFDIR, S_IFREG, lane_initramfs, read_newc, write_newc,
};

fn sample() -> Vec<Entry> {
    vec![
        Entry::dir("dev", 0o755),
        Entry::char_device("dev/console", 0o600, 5, 1),
        Entry::file("init", 0o755, b"#!lane-init with an odd length".to_vec()),
        Entry::dir("bin", 0o755),
        Entry::file("bin/lane-probe", 0o755, vec![0x7f, b'E', b'L', b'F', 0, 1]),
        Entry::file("lane.json", 0o644, b"{}".to_vec()),
        Entry::file("empty", 0o600, Vec::new()),
    ]
}

#[test]
fn the_writer_output_reads_back_to_the_same_entries_modes_and_bytes() {
    let archive = write_newc(&sample()).expect("the sample archive is written");

    assert_eq!(read_newc(&archive), Ok(sample()));
}

#[test]
fn the_archive_starts_with_the_newc_magic_and_ends_with_the_trailer_padded_to_four() {
    let archive = write_newc(&sample()).expect("the sample archive is written");

    assert_eq!(&archive[..6], b"070701");
    assert_eq!(archive.len() % 4, 0);
    let tail = String::from_utf8_lossy(&archive[archive.len() - 16..]).to_string();
    assert!(
        tail.contains("TRAILER!!!"),
        "the archive ends with the trailer: {tail:?}"
    );
}

#[test]
fn the_system_cpio_lists_the_same_names_in_the_same_order() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("sample.cpio");
    std::fs::write(&path, write_newc(&sample()).expect("written")).expect("archive saved");

    let output = Command::new("cpio")
        .arg("-it")
        .stdin(std::fs::File::open(&path).expect("archive opens"))
        .output()
        .expect("this test needs cpio on PATH, as macOS and the Ubuntu runner both have");
    assert!(
        output.status.success(),
        "cpio -it failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let listed: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let names: Vec<String> = sample().into_iter().map(|entry| entry.name).collect();
    assert_eq!(listed, names);
}

#[test]
fn an_absolute_name_is_refused() {
    let error = write_newc(&[Entry::file("/init", 0o755, Vec::new())]).expect_err("absolute");
    assert!(error.to_string().contains("/init"), "{error}");
}

#[test]
fn a_parent_component_is_refused() {
    let error =
        write_newc(&[Entry::file("bin/../../etc", 0o644, Vec::new())]).expect_err("escaping");
    assert!(error.to_string().contains("bin/../../etc"), "{error}");
}

#[test]
fn a_duplicate_name_is_refused() {
    let error = write_newc(&[
        Entry::file("init", 0o755, Vec::new()),
        Entry::file("init", 0o644, Vec::new()),
    ])
    .expect_err("duplicate");
    assert!(error.to_string().contains("init"), "{error}");
}

#[test]
fn an_empty_name_is_refused() {
    write_newc(&[Entry::file("", 0o644, Vec::new())]).expect_err("empty");
}

#[test]
fn a_truncated_archive_is_refused() {
    let archive = write_newc(&sample()).expect("written");

    read_newc(&archive[..archive.len() / 2]).expect_err("truncated");
}

#[test]
fn an_archive_with_the_wrong_magic_is_refused() {
    let mut archive = write_newc(&sample()).expect("written");
    archive[5] = b'2';

    read_newc(&archive).expect_err("crc magic is not newc");
}

#[test]
fn an_archive_without_a_trailer_is_refused() {
    let archive = write_newc(&sample()).expect("written");
    let without = &archive[..archive.len() - 124];

    read_newc(without).expect_err("no trailer");
}

#[test]
fn the_lane_initramfs_holds_init_the_probe_the_configuration_and_device_nodes() {
    let guest = common::guest_config("run-1", &["id"]);
    let archive = lane_initramfs(b"init bytes", b"probe bytes", &guest).expect("written");
    let entries = read_newc(&archive).expect("the initramfs reads back");

    let find = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("the initramfs has no `{name}`: {entries:?}"))
    };
    assert_eq!(find("init").mode, S_IFREG | 0o755);
    assert_eq!(find("init").data, b"init bytes");
    assert_eq!(find("bin/lane-probe").mode, S_IFREG | 0o755);
    assert_eq!(find("bin/lane-probe").data, b"probe bytes");
    let configuration = std::str::from_utf8(&find("lane.json").data).expect("UTF-8");
    assert_eq!(GuestConfig::from_json(configuration), Ok(guest));
    assert_eq!(find("lane.json").mode, S_IFREG | 0o644);
    let console = find("dev/console");
    assert_eq!(
        (
            console.mode & 0o170000,
            console.rdev_major,
            console.rdev_minor
        ),
        (S_IFCHR, 5, 1)
    );
    let null = find("dev/null");
    assert_eq!(
        (null.mode & 0o170000, null.rdev_major, null.rdev_minor),
        (S_IFCHR, 1, 3)
    );
    for dir in ["dev", "proc", "bin", "run"] {
        assert_eq!(find(dir).mode & 0o170000, S_IFDIR, "`{dir}` is a directory");
    }
    assert!(
        entries.iter().all(|entry| entry.uid == 0 && entry.gid == 0),
        "every initramfs entry is owned by root"
    );
}

#[test]
fn a_dot_component_an_empty_component_and_a_trailing_slash_are_refused() {
    for name in ["dev/./console", "dev//console", "dev/console/"] {
        let error = write_newc(&[
            Entry::dir("dev", 0o755),
            Entry::char_device("dev/console", 0o600, 5, 1),
            Entry::char_device(name, 0o600, 5, 1),
        ])
        .expect_err(name);
        assert!(error.to_string().contains(name), "{error}");
    }
}

#[test]
fn a_leading_dot_component_is_refused() {
    let error = write_newc(&[Entry::file("./init", 0o755, Vec::new())]).expect_err("./init");
    assert!(error.to_string().contains("./init"), "{error}");
}

#[test]
fn the_trailer_name_and_a_nul_are_refused_as_entry_names() {
    write_newc(&[Entry::file("TRAILER!!!", 0o644, Vec::new())]).expect_err("the trailer name");
    let with_nul = format!("in{}it", char::from(0));
    write_newc(&[Entry::file(&with_nul, 0o755, Vec::new())]).expect_err("a NUL");
}

#[test]
fn a_guest_configuration_that_cannot_serialize_is_refused_naming_the_mount() {
    let mut guest = common::guest_config("run-1", &["id"]);
    guest.mounts[0].path = PathBuf::from(OsStr::from_bytes(&[b'/', b'm', 0xff]));

    let error = lane_initramfs(b"init", b"probe", &guest).expect_err("not UTF-8");
    assert!(error.to_string().contains("mount"), "{error}");
}

#[test]
fn the_lane_initramfs_has_a_sys_directory_for_sysfs() {
    let guest = common::guest_config("run-1", &["id"]);
    let archive = lane_initramfs(b"init", b"probe", &guest).expect("written");
    let entries = read_newc(&archive).expect("the initramfs reads back");

    let sys = entries
        .iter()
        .find(|entry| entry.name == "sys")
        .unwrap_or_else(|| panic!("the initramfs has no `sys`: {entries:?}"));
    assert_eq!(sys.mode, S_IFDIR | 0o555);
}
