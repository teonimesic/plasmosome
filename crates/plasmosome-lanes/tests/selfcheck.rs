mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use plasmosome_lanes::selfcheck::{Check, passed, self_check};

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("the mode is set");
}

fn running_as_root(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let probe = common::write_file(dir, "owner-probe", b"");
    let uid = fs::metadata(&probe).expect("the probe file exists").uid();
    fs::remove_file(probe).expect("the probe file is removed");
    uid == 0
}

#[test]
fn an_unconfined_process_fails_the_check_and_leaves_no_file_behind() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let allowed = dir.path().join("allowed");
    let outside = dir.path().join("outside");
    common::write_file(&allowed, "placed", b"placed\n");
    common::write_file(&outside, "secret", b"secret\n");

    let checks = self_check(&allowed, &outside);

    assert_eq!(checks.len(), 4, "{checks:?}");
    assert_eq!(checks[0].name, "read inside");
    assert!(checks[0].holds(), "{:?}", checks[0]);
    assert!(
        !checks[1].holds(),
        "an unconfined write inside succeeded: {:?}",
        checks[1]
    );
    assert!(!passed(&checks));
    assert_eq!(
        fs::read_dir(&allowed).expect("listed").count(),
        1,
        "the probe file a successful write made was removed"
    );
    assert_eq!(fs::read_dir(&outside).expect("listed").count(), 1);
}

#[test]
fn permission_refusals_in_every_expected_place_pass_the_check() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    assert!(
        !running_as_root(dir.path()),
        "this test needs a non-root user: root ignores the directory modes it relies on"
    );
    let allowed = dir.path().join("allowed");
    let outside = dir.path().join("outside");
    common::write_file(&allowed, "placed", b"placed\n");
    fs::create_dir(&outside).expect("outside is created");
    set_mode(&allowed, 0o555);
    set_mode(&outside, 0o000);

    let checks = self_check(&allowed, &outside);

    set_mode(&allowed, 0o755);
    set_mode(&outside, 0o755);
    assert!(passed(&checks), "{checks:?}");
}

#[test]
fn a_missing_outside_directory_is_not_a_refusal() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let allowed = dir.path().join("allowed");
    common::write_file(&allowed, "placed", b"placed\n");
    set_mode(&allowed, 0o555);

    let checks = self_check(&allowed, &dir.path().join("missing"));

    set_mode(&allowed, 0o755);
    let read_outside = checks
        .iter()
        .find(|check| check.name == "read outside")
        .expect("the read outside ran");
    assert_eq!(read_outside.observed, Err("ENOENT".to_string()));
    assert!(!read_outside.holds());
    assert!(!passed(&checks));
}

#[test]
fn a_refusal_holds_only_as_eperm_or_eacces() {
    let check = |observed: Result<(), &str>| Check {
        name: "write outside",
        expected_allowed: false,
        observed: observed.map_err(str::to_string),
    };

    assert!(check(Err("EPERM")).holds());
    assert!(check(Err("EACCES")).holds());
    assert!(!check(Err("ENOENT")).holds());
    assert!(!check(Err("EROFS")).holds());
    assert!(!check(Ok(())).holds());
}

#[test]
fn an_expected_success_holds_only_when_it_succeeded() {
    let check = |observed: Result<(), &str>| Check {
        name: "read inside",
        expected_allowed: true,
        observed: observed.map_err(str::to_string),
    };

    assert!(check(Ok(())).holds());
    assert!(!check(Err("EPERM")).holds());
}

#[test]
fn fewer_than_four_checks_do_not_pass() {
    let ok = Check {
        name: "read inside",
        expected_allowed: true,
        observed: Ok(()),
    };

    assert!(!passed(&[]));
    assert!(!passed(&[ok.clone(), ok.clone(), ok]));
}
