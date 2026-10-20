mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use plasmosome_lanes::selfcheck::{Check, Phase, passed, self_check};

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

fn plain_directories(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let allowed = root.join("allowed");
    let outside = root.join("outside");
    common::write_file(&allowed, "placed", b"placed\n");
    common::write_file(&outside, "secret", b"secret\n");
    (allowed, outside)
}

fn check(name: &'static str, expected_allowed: bool, observed: Result<(), &str>) -> Check {
    Check {
        name,
        expected_allowed,
        observed: observed.map_err(str::to_string),
    }
}

#[test]
fn an_unconfined_process_fails_the_confined_check_and_leaves_no_file_behind() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (allowed, outside) = plain_directories(dir.path());

    let checks = self_check(Phase::Confined, &allowed, &outside);

    assert_eq!(checks.len(), 4, "{checks:?}");
    assert_eq!(checks[0].name, "read inside");
    assert!(checks[0].holds(), "{:?}", checks[0]);
    assert!(
        !checks[1].holds(),
        "an unconfined write inside succeeded: {:?}",
        checks[1]
    );
    assert!(!passed(Phase::Confined, &checks));
    assert_eq!(
        fs::read_dir(&allowed).expect("listed").count(),
        1,
        "the probe file a successful write made was removed"
    );
    assert_eq!(fs::read_dir(&outside).expect("listed").count(), 1);
}

#[test]
fn the_unconfined_control_passes_when_all_four_operations_succeed() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (allowed, outside) = plain_directories(dir.path());

    let checks = self_check(Phase::Unconfined, &allowed, &outside);

    let names: Vec<_> = checks.iter().map(|check| check.name).collect();
    assert_eq!(
        names,
        [
            "read inside",
            "write inside",
            "read outside",
            "write outside"
        ]
    );
    assert!(
        checks.iter().all(|check| check.expected_allowed),
        "{checks:?}"
    );
    assert!(passed(Phase::Unconfined, &checks), "{checks:?}");
    assert_eq!(fs::read_dir(&allowed).expect("listed").count(), 1);
    assert_eq!(fs::read_dir(&outside).expect("listed").count(), 1);
}

#[test]
fn file_modes_that_refuse_without_a_sandbox_fail_both_the_control_and_the_check() {
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

    let control = self_check(Phase::Unconfined, &allowed, &outside);
    let confined = self_check(Phase::Confined, &allowed, &outside);

    set_mode(&allowed, 0o755);
    set_mode(&outside, 0o755);
    assert!(
        !passed(Phase::Unconfined, &control),
        "the control passed with directories the user cannot write: {control:?}"
    );
    assert!(
        !passed(Phase::Confined, &confined),
        "EACCES from file modes counted as the sandbox refusing: {confined:?}"
    );
}

#[test]
fn a_missing_outside_directory_fails_the_control_and_is_not_a_refusal() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let allowed = dir.path().join("allowed");
    common::write_file(&allowed, "placed", b"placed\n");
    let missing = dir.path().join("missing");

    let control = self_check(Phase::Unconfined, &allowed, &missing);
    set_mode(&allowed, 0o555);
    let confined = self_check(Phase::Confined, &allowed, &missing);
    set_mode(&allowed, 0o755);

    assert!(!passed(Phase::Unconfined, &control), "{control:?}");
    let read_outside = confined
        .iter()
        .find(|check| check.name == "read outside")
        .expect("the read outside ran");
    assert_eq!(read_outside.observed, Err("ENOENT".to_string()));
    assert!(!read_outside.holds());
    assert!(!passed(Phase::Confined, &confined));
}

#[test]
fn a_refusal_holds_only_as_eperm() {
    let refused = |observed| check("write outside", false, observed);

    assert!(refused(Err("EPERM")).holds());
    assert!(!refused(Err("EACCES")).holds());
    assert!(!refused(Err("ENOENT")).holds());
    assert!(!refused(Err("EROFS")).holds());
    assert!(!refused(Ok(())).holds());
}

#[test]
fn an_expected_success_holds_only_when_it_succeeded() {
    let allowed = |observed| check("read inside", true, observed);

    assert!(allowed(Ok(())).holds());
    assert!(!allowed(Err("EPERM")).holds());
}

#[test]
fn eperm_in_every_expected_place_passes_the_confined_check() {
    let checks = [
        check("read inside", true, Ok(())),
        check("write inside", false, Err("EPERM")),
        check("read outside", false, Err("EPERM")),
        check("write outside", false, Err("EPERM")),
    ];

    assert!(passed(Phase::Confined, &checks));
    assert!(
        !passed(Phase::Unconfined, &checks),
        "confined results must not pass as the unconfined control"
    );
}

#[test]
fn fewer_than_four_checks_do_not_pass_either_phase() {
    let ok = check("read inside", true, Ok(()));

    for phase in [Phase::Unconfined, Phase::Confined] {
        assert!(!passed(phase, &[]));
        assert!(!passed(phase, &[ok.clone(), ok.clone(), ok.clone()]));
    }
}
