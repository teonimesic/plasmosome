#![cfg(target_os = "macos")]

mod common;

use plasmosome_lanes::bundle::{PINNED_BUNDLE_ID, verify_bundle};
use plasmosome_lanes::krun::Libkrun;

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
