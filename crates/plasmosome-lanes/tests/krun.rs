#![cfg(target_os = "macos")]

mod common;

use plasmosome_lanes::bundle::{PINNED_BUNDLE_ID, verify_bundle};
use std::path::Path;

use plasmosome_lanes::krun::{Libkrun, in_shared_cache};

#[test]
fn libkrun_opens_only_from_the_pinned_bundle() {
    let fixture = common::bundle_fixture();
    let verified = verify_bundle(fixture.path(), &fixture.id).expect("the fixture verifies");

    let error = match Libkrun::open(&verified) {
        Ok(_) => panic!("a bundle that is not the pinned one was loaded"),
        Err(error) => error.to_string(),
    };

    assert!(
        error.contains(&fixture.id) && error.contains(PINNED_BUNDLE_ID),
        "the refusal should name both bundles: {error}"
    );
}

#[test]
fn system_libraries_are_in_the_shared_cache_and_this_test_binary_is_not() {
    assert!(in_shared_cache(Path::new("/usr/lib/libSystem.B.dylib")));
    assert!(in_shared_cache(Path::new(
        "/System/Library/Frameworks/Hypervisor.framework/Versions/A/Hypervisor"
    )));
    let me = std::env::current_exe().expect("the test binary has a path");
    assert!(!in_shared_cache(&me));
    assert!(!in_shared_cache(Path::new(
        "/System/Volumes/Data/tmp/evil.dylib"
    )));
}
