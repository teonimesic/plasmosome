mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use std::path::PathBuf;

use plasmosome_lanes::bundle::{
    BundleError, KERNEL, LIBKRUN, PINNED_BUNDLE_ID, is_inside, sha256_hex, unverified_images,
    verify_bundle,
};
use serde_json::json;

use common::BundleFixture;

fn refused_naming(dir: &BundleFixture, path: &str) -> BundleError {
    refused_against(dir.path(), &dir.id, path)
}

fn refused_against(dir: &Path, pinned: &str, path: &str) -> BundleError {
    match verify_bundle(dir, pinned) {
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

    let bundle = verify_bundle(dir.path(), &dir.id).expect("the fixture bundle matches its sums");

    assert_eq!(bundle.path(LIBKRUN), Some(dir.path().join(LIBKRUN)));
    assert_eq!(bundle.path(KERNEL), Some(dir.path().join(KERNEL)));
    assert_eq!(
        bundle.digest(LIBKRUN),
        Some(common::sha256(b"not really a dylib").as_str())
    );
    assert_eq!(bundle.path("manifest.json"), None);
    assert_eq!(bundle.id(), dir.id);
}

#[test]
fn the_pinned_bundle_is_the_one_on_this_machine() {
    assert_eq!(PINNED_BUNDLE_ID, "eed141b574337d99");
}

#[test]
fn a_self_consistent_bundle_with_another_id_is_refused_against_the_pin() {
    let dir = common::bundle_fixture();

    let error = refused_against(dir.path(), PINNED_BUNDLE_ID, "manifest.json");

    assert!(
        error.reason.contains(&dir.id) && error.reason.contains(PINNED_BUNDLE_ID),
        "the refusal should name both bundles: {error}"
    );
}

#[test]
fn a_manifest_that_does_not_hash_to_the_directory_name_is_refused() {
    let dir = common::bundle_fixture();
    dir.write_manifest(&json!({
        "bundle_format": "plasmosome-runtime-bundle/1",
        "sha256sums_sha256": dir.sums_digest(),
        "edited": true,
    }));

    refused_naming(&dir, "manifest.json");
}

#[test]
fn sums_whose_digest_the_manifest_does_not_list_are_refused() {
    let dir = common::bundle_fixture();
    let mut text = fs::read_to_string(dir.path().join("SHA256SUMS")).expect("sums");
    text.push_str(&format!("{}  boot/extra\n", common::sha256(b"extra")));
    fs::write(dir.path().join("SHA256SUMS"), text).expect("sums rewritten");
    common::write_file(dir.path(), "boot/extra", b"extra");

    let error = refused_naming(&dir, "SHA256SUMS");
    assert!(error.reason.contains("manifest.json"), "{error}");
}

#[test]
fn a_manifest_with_no_sums_digest_is_refused() {
    let mut dir = common::bundle_fixture();
    dir.write_manifest(&json!({"bundle_format": "plasmosome-runtime-bundle/1"}));
    dir.rename_to_manifest();

    refused_naming(&dir, "manifest.json");
}

#[test]
fn a_symlinked_directory_inside_the_bundle_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    let moved = dir.parent().join("boot-elsewhere");
    fs::rename(dir.path().join("boot"), &moved).expect("boot/ is moved out of the bundle");
    symlink(&moved, dir.path().join("boot")).expect("boot becomes a symlink to the moved copy");

    refused_naming(&dir, "boot");
}

#[test]
fn a_symlink_anywhere_in_the_tree_is_refused_even_when_unlisted() {
    let dir = common::bundle_fixture();
    symlink("/etc/hosts", dir.path().join("base/hosts")).expect("the fixture symlink is created");

    refused_naming(&dir, "base/hosts");
}

#[test]
fn a_fifo_in_the_tree_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    let status = std::process::Command::new("mkfifo")
        .arg(dir.path().join("base/pipe"))
        .status()
        .expect("mkfifo runs");
    assert!(status.success());

    refused_naming(&dir, "base/pipe");
}

#[test]
fn a_bundle_directory_that_is_a_symlink_is_refused() {
    let dir = common::bundle_fixture();
    let real = dir.parent().join(format!("{}-real", dir.id));
    fs::rename(dir.path(), &real).expect("the bundle is moved");
    symlink(&real, dir.path()).expect("the bundle's name becomes a symlink to it");

    refused_against(dir.path(), &dir.id, ".");
}

#[test]
fn a_relative_bundle_directory_is_refused() {
    let dir = common::bundle_fixture();

    refused_against(Path::new(&dir.id), &dir.id, ".");
}

#[test]
fn one_changed_byte_in_libkrun_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    change_one_byte(&dir.path().join(LIBKRUN));

    refused_naming(&dir, LIBKRUN);
}

#[test]
fn one_changed_byte_in_the_kernel_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    change_one_byte(&dir.path().join(KERNEL));

    refused_naming(&dir, KERNEL);
}

#[test]
fn one_changed_byte_in_any_listed_file_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    change_one_byte(&dir.path().join("base/rootfs.raw"));

    refused_naming(&dir, "base/rootfs.raw");
}

#[test]
fn a_listed_file_that_is_missing_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    fs::remove_file(dir.path().join("boot/initramfs")).expect("the fixture file is removed");

    refused_naming(&dir, "boot/initramfs");
}

#[test]
fn an_extra_unlisted_file_under_lib_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    common::write_file(dir.path(), "lib/libkrunfw.5.dylib", b"unlisted code");

    refused_naming(&dir, "lib/libkrunfw.5.dylib");
}

#[test]
fn an_unlisted_file_nested_under_lib_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    common::write_file(dir.path(), "lib/plugins/extra.dylib", b"unlisted code");

    refused_naming(&dir, "lib/plugins/extra.dylib");
}

#[test]
fn an_unlisted_symlink_under_lib_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    symlink(
        dir.path().join(LIBKRUN),
        dir.path().join("lib/libkrun.dylib"),
    )
    .expect("the fixture symlink is created");

    refused_naming(&dir, "lib/libkrun.dylib");
}

#[test]
fn a_listed_path_that_is_a_symlink_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    let kernel = dir.path().join(KERNEL);
    let real = dir.path().join("boot/kernel.real");
    fs::rename(&kernel, &real).expect("the kernel is moved aside");
    symlink(&real, &kernel).expect("the kernel becomes a symlink to the same bytes");

    refused_naming(&dir, KERNEL);
}

#[test]
fn a_sums_line_for_a_path_outside_the_bundle_is_refused_naming_it() {
    let mut dir = common::bundle_fixture();
    let mut lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    lines.push((common::sha256(b"outside"), "../outside"));
    common::write_sums(dir.path(), &lines);
    dir.reseal();

    refused_naming(&dir, "../outside");
}

#[test]
fn an_absolute_sums_path_is_refused_naming_it() {
    let mut dir = common::bundle_fixture();
    let mut lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    lines.push((common::sha256(b"hosts"), "/etc/hosts"));
    common::write_sums(dir.path(), &lines);
    dir.reseal();

    refused_naming(&dir, "/etc/hosts");
}

#[test]
fn a_duplicated_sums_path_is_refused_naming_it() {
    let mut dir = common::bundle_fixture();
    let mut lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    lines.push((common::sha256(b"not really a kernel"), KERNEL));
    common::write_sums(dir.path(), &lines);
    dir.reseal();

    refused_naming(&dir, KERNEL);
}

#[test]
fn a_malformed_sums_line_is_refused_naming_the_sums_file() {
    let mut dir = common::bundle_fixture();
    let mut text = fs::read_to_string(dir.path().join("SHA256SUMS")).expect("sums");
    text.push_str("not-a-digest  boot/extra\n");
    fs::write(dir.path().join("SHA256SUMS"), text).expect("sums rewritten");
    dir.reseal();

    let error = refused_naming(&dir, "SHA256SUMS");
    assert!(
        error.reason.contains("line 5"),
        "the refusal should say which line: {error}"
    );
}

#[test]
fn an_uppercase_digest_is_refused_as_malformed() {
    let mut dir = common::bundle_fixture();
    let lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .map(|(path, bytes)| (common::sha256(bytes).to_uppercase(), *path))
        .collect();
    common::write_sums(dir.path(), &lines);
    dir.reseal();

    refused_naming(&dir, "SHA256SUMS");
}

#[test]
fn a_missing_sums_file_is_refused_naming_it() {
    let dir = common::bundle_fixture();
    fs::remove_file(dir.path().join("SHA256SUMS")).expect("sums removed");

    refused_naming(&dir, "SHA256SUMS");
}

#[test]
fn sums_that_do_not_list_libkrun_are_refused_naming_it() {
    let mut dir = common::bundle_fixture();
    fs::remove_file(dir.path().join(LIBKRUN)).expect("libkrun removed");
    let lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .filter(|(path, _)| *path != LIBKRUN)
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    common::write_sums(dir.path(), &lines);
    dir.reseal();

    refused_naming(&dir, LIBKRUN);
}

#[test]
fn sums_that_do_not_list_the_kernel_are_refused_naming_it() {
    let mut dir = common::bundle_fixture();
    fs::remove_file(dir.path().join(KERNEL)).expect("kernel removed");
    let lines: Vec<(String, &str)> = common::BUNDLE_FILES
        .iter()
        .filter(|(path, _)| *path != KERNEL)
        .map(|(path, bytes)| (common::sha256(bytes), *path))
        .collect();
    common::write_sums(dir.path(), &lines);
    dir.reseal();

    refused_naming(&dir, KERNEL);
}

const EXECUTABLE: &str = "/private/tmp/target/debug/lane-vmm";

fn cached(path: &Path) -> bool {
    [
        "/usr/lib/libSystem.B.dylib",
        "/usr/lib/libiconv.2.dylib",
        "/System/Library/Frameworks/Hypervisor.framework/Versions/A/Hypervisor",
    ]
    .iter()
    .any(|image| Path::new(image) == path)
}

fn system_and_bundle_images(libkrun: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from(EXECUTABLE),
        PathBuf::from("/usr/lib/libSystem.B.dylib"),
        PathBuf::from("/System/Library/Frameworks/Hypervisor.framework/Versions/A/Hypervisor"),
        libkrun.to_path_buf(),
        PathBuf::from("/usr/lib/libiconv.2.dylib"),
        PathBuf::from("/usr/lib/dyld"),
    ]
}

fn unverified(images: &[PathBuf], libkrun: &Path) -> Vec<PathBuf> {
    unverified_images(images, Path::new(EXECUTABLE), libkrun, cached)
}

#[test]
fn the_executable_the_verified_libkrun_the_shared_cache_and_usr_lib_are_accepted() {
    let dir = common::bundle_fixture();
    let libkrun = dir.path().join(LIBKRUN);

    assert_eq!(
        unverified(&system_and_bundle_images(&libkrun), &libkrun),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn a_libkrunfw_found_through_the_leftover_search_path_is_reported() {
    let dir = common::bundle_fixture();
    let libkrun = dir.path().join(LIBKRUN);
    let found = PathBuf::from("/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib");
    let mut images = system_and_bundle_images(&libkrun);
    images.push(found.clone());

    assert_eq!(unverified(&images, &libkrun), vec![found]);
}

#[test]
fn a_libkrun_loaded_from_somewhere_else_is_reported() {
    let dir = common::bundle_fixture();
    let libkrun = dir.path().join(LIBKRUN);
    let other = PathBuf::from("/opt/homebrew/lib/libkrun.1.19.4.dylib");
    let mut images = system_and_bundle_images(&libkrun);
    images[3] = other.clone();

    assert_eq!(unverified(&images, &libkrun), vec![other]);
}

#[test]
fn a_library_loaded_before_the_executable_is_reported_and_not_taken_for_it() {
    let dir = common::bundle_fixture();
    let libkrun = dir.path().join(LIBKRUN);
    let inserted = PathBuf::from("/tmp/inserted.dylib");
    let mut images = system_and_bundle_images(&libkrun);
    images.insert(0, inserted.clone());

    assert_eq!(unverified(&images, &libkrun), vec![inserted]);
}

#[test]
fn a_system_path_outside_the_shared_cache_is_reported() {
    let dir = common::bundle_fixture();
    let libkrun = dir.path().join(LIBKRUN);
    let data = PathBuf::from("/System/Volumes/Data/tmp/evil.dylib");
    let framework = PathBuf::from("/System/Library/Frameworks/Evil.framework/Evil");
    let mut images = system_and_bundle_images(&libkrun);
    images.push(data.clone());
    images.push(framework.clone());

    assert_eq!(unverified(&images, &libkrun), vec![data, framework]);
}

#[test]
fn a_path_that_only_starts_like_usr_lib_is_reported() {
    let dir = common::bundle_fixture();
    let libkrun = dir.path().join(LIBKRUN);
    let lookalikes = [
        "/usr/libexec-not-really/evil.dylib",
        "/usr/lib/../../tmp/evil.dylib",
        "/usr/lib/./evil.dylib",
        "/usr/lib//evil.dylib",
        "/Systemic/evil.dylib",
    ]
    .map(PathBuf::from);
    let mut images = system_and_bundle_images(&libkrun);
    images.extend(lookalikes.iter().cloned());

    assert_eq!(unverified(&images, &libkrun), lookalikes.to_vec());
}

#[test]
fn a_file_in_the_bundle_is_inside_it_by_any_spelling_and_one_outside_is_not() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let bundle = scratch.path().join("bundle");
    let image = common::write_file(&bundle, "base/rootfs.raw", b"disk");
    let outside = common::write_file(scratch.path(), "copy/rootfs.raw", b"disk");
    let link = scratch.path().join("link.raw");
    symlink(&image, &link).expect("a symlink to the bundle's image");

    assert!(is_inside(&bundle, &image));
    assert!(is_inside(&bundle, &bundle.join("base/../base/rootfs.raw")));
    assert!(
        is_inside(&bundle, &link),
        "a symlink into the bundle is inside it"
    );
    assert!(!is_inside(&bundle, &outside));
    assert!(!is_inside(&bundle, &scratch.path().join("missing.raw")));
    assert!(
        !is_inside(&bundle, &scratch.path().join("bundle-other/rootfs.raw")),
        "a sibling whose name starts with the bundle's is not inside it"
    );
}
