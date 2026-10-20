mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use plasmosome_lanes::bundle::{BundleError, KERNEL, LIBKRUN, sha256_hex, verify_bundle};

fn refused_naming(dir: &Path, path: &str) -> BundleError {
    match verify_bundle(dir) {
        Ok(bundle) => panic!("expected a refusal naming `{path}`, the bundle passed: {bundle:?}"),
        Err(error) => {
            assert_eq!(
                error.path, path,
                "the refusal names the wrong path: {error}"
            );
            assert!(
                error.to_string().contains(path),
                "the message should name `{path}`: {error}"
            );
            error
        }
    }
}

fn change_one_byte(path: &Path) {
    let mut bytes = fs::read(path).expect("the fixture file is readable");
    bytes[0] ^= 1;
    fs::write(path, bytes).expect("the fixture file is rewritten");
}

#[test]
fn sha256_hex_matches_the_published_answer_for_abc() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn a_bundle_that_matches_its_sums_passes_and_names_its_files() {
    let dir = common::bundle_fixture();

    let bundle = verify_bundle(dir.path()).expect("the fixture bundle matches its sums");

    assert_eq!(bundle.path(LIBKRUN), Some(dir.path().join(LIBKRUN)));
    assert_eq!(bundle.path(KERNEL), Some(dir.path().join(KERNEL)));
    assert_eq!(
        bundle.digest(LIBKRUN),
        Some(common::sha256(b"not really a dylib").as_str())
    );
    assert_eq!(bundle.path("manifest.json"), None);
}

#[test]
fn one_changed_byte_in_libkrun_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    change_one_byte(&dir.path().join(LIBKRUN));

    refused_naming(dir.path(), LIBKRUN);
}

#[test]
fn one_changed_byte_in_the_kernel_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    change_one_byte(&dir.path().join(KERNEL));

    refused_naming(dir.path(), KERNEL);
}

#[test]
fn one_changed_byte_in_any_listed_file_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    change_one_byte(&dir.path().join("base/rootfs.raw"));

    refused_naming(dir.path(), "base/rootfs.raw");
}

#[test]
fn a_listed_file_that_is_missing_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    fs::remove_file(dir.path().join("boot/initramfs")).expect("the fixture file is removed");

    refused_naming(dir.path(), "boot/initramfs");
}

#[test]
fn an_extra_unlisted_file_under_lib_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    common::write_file(dir.path(), "lib/libkrunfw.5.dylib", b"unlisted code");

    refused_naming(dir.path(), "lib/libkrunfw.5.dylib");
}

#[test]
fn an_unlisted_file_nested_under_lib_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    common::write_file(dir.path(), "lib/plugins/extra.dylib", b"unlisted code");

    refused_naming(dir.path(), "lib/plugins/extra.dylib");
}

#[test]
fn an_unlisted_symlink_under_lib_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    symlink(
        dir.path().join(LIBKRUN),
        dir.path().join("lib/libkrun.dylib"),
    )
    .expect("the fixture symlink is created");

    refused_naming(dir.path(), "lib/libkrun.dylib");
}

#[test]
fn a_listed_path_that_is_a_symlink_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    let kernel = dir.path().join(KERNEL);
    let real = dir.path().join("boot/kernel.real");
    fs::rename(&kernel, &real).expect("the kernel is moved aside");
    symlink(&real, &kernel).expect("the kernel becomes a symlink to the same bytes");

    refused_naming(dir.path(), KERNEL);
}

#[test]
fn a_sums_line_for_a_path_outside_the_bundle_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    let mut lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    lines.push((common::sha256(b"outside"), "../outside"));
    common::write_sums(dir.path(), &lines);

    refused_naming(dir.path(), "../outside");
}

#[test]
fn an_absolute_sums_path_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    let mut lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    lines.push((common::sha256(b"hosts"), "/etc/hosts"));
    common::write_sums(dir.path(), &lines);

    refused_naming(dir.path(), "/etc/hosts");
}

#[test]
fn a_duplicated_sums_path_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    let mut lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    lines.push((common::sha256(b"not really a kernel"), KERNEL));
    common::write_sums(dir.path(), &lines);

    refused_naming(dir.path(), KERNEL);
}

#[test]
fn a_malformed_sums_line_is_refused_naming_the_sums_file() {
    let dir = common::bundle_fixture();
    let mut text = fs::read_to_string(dir.path().join("SHA256SUMS")).expect("sums");
    text.push_str("not-a-digest  boot/extra\n");
    fs::write(dir.path().join("SHA256SUMS"), text).expect("sums rewritten");

    let error = refused_naming(dir.path(), "SHA256SUMS");
    assert!(
        error.reason.contains("line 5"),
        "the refusal should say which line: {error}"
    );
}

#[test]
fn an_uppercase_digest_is_refused_as_malformed() {
    let dir = common::bundle_fixture();
    let lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes).to_uppercase(), *path))
        .collect();
    common::write_sums(dir.path(), &lines);

    refused_naming(dir.path(), "SHA256SUMS");
}

#[test]
fn a_missing_sums_file_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    fs::remove_file(dir.path().join("SHA256SUMS")).expect("sums removed");

    refused_naming(dir.path(), "SHA256SUMS");
}

#[test]
fn sums_that_do_not_list_libkrun_are_refused_naming_it() {
    let dir = common::bundle_fixture();
    fs::remove_file(dir.path().join(LIBKRUN)).expect("libkrun removed");
    let lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .filter(|(path, _)| *path != LIBKRUN)
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    common::write_sums(dir.path(), &lines);

    refused_naming(dir.path(), LIBKRUN);
}

#[test]
fn sums_that_do_not_list_the_kernel_are_refused_naming_it() {
    let dir = common::bundle_fixture();
    fs::remove_file(dir.path().join(KERNEL)).expect("kernel removed");
    let lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .filter(|(path, _)| *path != KERNEL)
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    common::write_sums(dir.path(), &lines);

    refused_naming(dir.path(), KERNEL);
}
