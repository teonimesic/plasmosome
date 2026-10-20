mod common;

use std::path::{Path, PathBuf};

use plasmosome_lanes::ext4::{ext4_from_dir, ext4_read};

fn tool_dir() -> PathBuf {
    let dir = if cfg!(target_os = "macos") {
        Path::new("/opt/homebrew/opt/e2fsprogs/sbin")
    } else {
        Path::new("/usr/sbin")
    };
    assert!(
        dir.join("mke2fs").exists() && dir.join("debugfs").exists(),
        "this test needs e2fsprogs' mke2fs and debugfs in {}",
        dir.display()
    );
    dir.to_path_buf()
}

#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "needs e2fsprogs from Homebrew, which a hosted macOS runner does not have"
)]
fn a_staged_tree_round_trips_through_an_ext4_image() {
    let tools = tool_dir();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let staged = scratch.path().join("staged");
    common::write_file(&staged, "etc/lane", b"lane fixture\n");
    common::write_file(&staged, "opt/tool/bin/run", &[0u8, 1, 2, 3, 255]);
    let image = scratch.path().join("root.ext4");

    ext4_from_dir(&tools, &staged, &image).expect("mke2fs builds the image");

    assert_eq!(
        ext4_read(&tools, &image, "/etc/lane").expect("read back"),
        b"lane fixture\n"
    );
    assert_eq!(
        ext4_read(&tools, &image, "/opt/tool/bin/run").expect("read back"),
        vec![0u8, 1, 2, 3, 255]
    );
}

#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "needs e2fsprogs from Homebrew, which a hosted macOS runner does not have"
)]
fn reading_a_path_the_image_lacks_is_refused() {
    let tools = tool_dir();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let staged = scratch.path().join("staged");
    common::write_file(&staged, "etc/lane", b"lane fixture\n");
    let image = scratch.path().join("root.ext4");
    ext4_from_dir(&tools, &staged, &image).expect("mke2fs builds the image");

    let error = ext4_read(&tools, &image, "/etc/missing").expect_err("no such path");
    assert!(error.to_string().contains("/etc/missing"), "{error}");
}

#[test]
fn a_missing_tool_directory_is_refused_naming_mke2fs() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let staged = scratch.path().join("staged");
    common::write_file(&staged, "etc/lane", b"lane\n");
    let image = scratch.path().join("root.ext4");

    let error = ext4_from_dir(&scratch.path().join("no-tools"), &staged, &image)
        .expect_err("there is no mke2fs");
    assert!(error.to_string().contains("mke2fs"), "{error}");
    assert!(!image.exists(), "no image is left behind");
}

#[test]
fn a_missing_tool_directory_is_refused_naming_debugfs() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let image = common::write_file(scratch.path(), "root.ext4", b"");

    let error = ext4_read(&scratch.path().join("no-tools"), &image, "/etc/lane")
        .expect_err("there is no debugfs");
    assert!(error.to_string().contains("debugfs"), "{error}");
}
