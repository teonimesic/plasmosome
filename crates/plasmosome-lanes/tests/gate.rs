use std::fs;
use std::io;
use std::path::PathBuf;

use plasmosome_lanes::console::ProbeResult;
use plasmosome_lanes::gate::{
    self, Expected, SourceCase, Tally, Trial, Verdict, detach_probes, judge_m1, judge_m2, judge_m3,
    judge_m4, judge_m5, judge_r1, judge_r2, judge_r3, judge_r4_in_flight, judge_r4_thread,
    observe_names, refuses_source,
};
use serde_json::{Value, json};

fn framed(id: &str, uid: u32, mut line: Value) -> ProbeResult {
    if line.get("id_u").is_none() {
        line["id_u"] = json!(uid);
    }
    ProbeResult {
        probe: id.to_string(),
        uid,
        exit: Some(0),
        result: Some(line),
    }
}

fn stat(uid: u32, gid: u32) -> Value {
    json!({"probe": "stat", "ok": true, "kind": "dir", "mode": "0755", "uid": uid, "gid": gid,
           "size": 96, "nlink": 3, "dev": 23, "ino": 7})
}

fn refused(probe: &str, errno: &str) -> Value {
    json!({"probe": probe, "ok": false, "errno": errno})
}

fn is_fail(verdict: &Verdict, needle: &str) -> bool {
    matches!(verdict, Verdict::Fail(reason) if reason.contains(needle))
}

fn is_not_run(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::NotRun(_))
}

fn m1(owner: (u32, u32)) -> Vec<ProbeResult> {
    vec![
        framed("m1-share", 1000, stat(owner.0, owner.1)),
        framed("m1-file", 1000, stat(owner.0, owner.1)),
    ]
}

#[test]
fn m1_passes_when_the_workload_sees_root_ownership() {
    assert_eq!(judge_m1(&m1((0, 0))), Verdict::Pass);
}

#[test]
fn m1_fails_when_the_workload_sees_the_hosts_owner_or_itself() {
    assert!(is_fail(&judge_m1(&m1((501, 20))), "501:20"));
    assert!(is_fail(&judge_m1(&m1((1000, 1000))), "1000:1000"));
    let mut mixed = m1((0, 0));
    mixed[1] = framed("m1-file", 1000, stat(0, 20));
    assert!(is_fail(&judge_m1(&mixed), "m1-file"));
}

#[test]
fn m1_is_not_run_without_a_workload_stat() {
    let mut results = m1((0, 0));
    results[0] = framed("m1-share", 1000, refused("stat", "ENOENT"));
    assert!(is_not_run(&judge_m1(&results)));
    assert!(is_not_run(&judge_m1(&m1((0, 0))[..1])));
    let as_root = vec![
        framed("m1-share", 0, stat(0, 0)),
        framed("m1-file", 0, stat(0, 0)),
    ];
    assert!(is_not_run(&judge_m1(&as_root)));
    let mut lying = m1((0, 0));
    lying[0].result.as_mut().expect("a line")["id_u"] = json!(0);
    assert!(is_not_run(&judge_m1(&lying)));
    let mut framed_as_root = m1((0, 0));
    framed_as_root[0].uid = 0;
    assert!(
        matches!(judge_m1(&framed_as_root), Verdict::NotRun(reason) if reason.contains("ran as uid 0"))
    );
}

fn mountinfo(options: &str) -> Value {
    json!({"probe": "mountinfo", "ok": true, "mounts": [
        {"mount_point": "/", "fstype": "rootfs", "mount_options": "rw", "source": "rootfs",
         "root": "/", "optional_fields": [], "super_options": "rw"},
        {"mount_point": gate::SHARE_MOUNT, "fstype": "virtiofs", "mount_options": options,
         "source": gate::SHARE_TAG, "root": "/", "optional_fields": [], "super_options": "rw"}
    ]})
}

fn m2(workload: Value, root: Value, options: &str) -> Vec<ProbeResult> {
    vec![
        framed("m2-workload-create", 1000, workload),
        framed("m2-root-create", 0, root),
        framed("m2-mountinfo", 1000, mountinfo(options)),
    ]
}

fn created() -> Value {
    json!({"probe": "create", "ok": true, "bytes": 11})
}

#[test]
fn m2_passes_on_eacces_for_the_workload_and_erofs_for_root_without_ro() {
    let results = m2(
        refused("create", "EACCES"),
        refused("create", "EROFS"),
        "rw,relatime",
    );
    assert_eq!(judge_m2(&results), Verdict::Pass);
}

#[test]
fn m2_fails_on_a_ro_mount_a_writable_share_or_another_errno() {
    let ro = m2(
        refused("create", "EROFS"),
        refused("create", "EROFS"),
        "ro,relatime",
    );
    assert!(is_fail(&judge_m2(&ro), "EROFS"));
    let ro_option_only = m2(
        refused("create", "EACCES"),
        refused("create", "EROFS"),
        "ro,relatime",
    );
    assert!(is_fail(&judge_m2(&ro_option_only), "ro"));
    let writable = m2(refused("create", "EACCES"), created(), "rw,relatime");
    assert!(is_fail(&judge_m2(&writable), "m2-root-create"));
    let eperm = m2(
        refused("create", "EPERM"),
        refused("create", "EROFS"),
        "rw,relatime",
    );
    assert!(is_fail(&judge_m2(&eperm), "EPERM"));
    let root_eacces = m2(
        refused("create", "EACCES"),
        refused("create", "EACCES"),
        "rw,relatime",
    );
    assert!(is_fail(&judge_m2(&root_eacces), "m2-root-create"));
    let workload_wrote = m2(created(), refused("create", "EROFS"), "rw,relatime");
    assert!(is_fail(&judge_m2(&workload_wrote), "m2-workload-create"));
}

#[test]
fn m2_is_not_run_when_the_share_is_not_mounted_or_a_line_ran_as_the_wrong_uid() {
    let mut unmounted = m2(
        refused("create", "EACCES"),
        refused("create", "EROFS"),
        "rw",
    );
    unmounted[2] = framed(
        "m2-mountinfo",
        1000,
        json!({"probe": "mountinfo", "ok": true, "mounts": []}),
    );
    assert!(is_not_run(&judge_m2(&unmounted)));
    let mut eacces_as_root = m2(
        refused("create", "EACCES"),
        refused("create", "EROFS"),
        "rw",
    );
    eacces_as_root[0] = framed("m2-workload-create", 0, refused("create", "EACCES"));
    assert!(is_not_run(&judge_m2(&eacces_as_root)));
    let mut erofs_as_workload = m2(
        refused("create", "EACCES"),
        refused("create", "EROFS"),
        "rw",
    );
    erofs_as_workload[1] = framed("m2-root-create", 1000, refused("create", "EROFS"));
    assert!(is_not_run(&judge_m2(&erofs_as_workload)));
}

fn expected(sha: &str, bytes: u64) -> Expected {
    Expected {
        sha256: sha.to_string(),
        bytes,
    }
}

fn data() -> Expected {
    expected("aa", 5)
}

fn tool() -> Expected {
    expected("bb", 9)
}

fn released(name: &str, kind: &str, held: &Expected) -> Value {
    json!({"probe": "release", "ok": true, "name": name, "kind": kind,
           "bytes": held.bytes, "sha256": held.sha256})
}

fn listing(names: &[&str]) -> Value {
    json!({"probe": "list", "ok": true, "names": names})
}

fn cwd_release(listing: Value, lookups: &[(&str, Value)]) -> Value {
    let lookups: Vec<Value> = lookups
        .iter()
        .map(|(name, result)| json!({"name": name, "result": result}))
        .collect();
    json!({"probe": "release", "ok": true, "name": "cwd", "kind": "cwd",
           "listing": listing, "lookups": lookups})
}

fn gone() -> Value {
    refused("lookup", "ENOENT")
}

fn found() -> Value {
    json!({"probe": "lookup", "ok": true, "kind": "dir"})
}

fn ready(probe: &str, kind: &str) -> Value {
    json!({"probe": probe, "ok": true, "name": kind, "kind": kind, "pid": 200})
}

fn detach(waited_ms: Option<u64>) -> Vec<ProbeResult> {
    let mut results = vec![
        framed("m3-hold-fd", 1000, ready("hold-fd", "fd")),
        framed("m3-hold-map", 1000, ready("hold-map", "map")),
        framed("m3-hold-exec", 1000, ready("hold-exec", "exec")),
        framed(gate::DETACH_AFTER, 1000, ready("hold-cwd", "cwd")),
        framed(
            "detach-await",
            1000,
            json!({"probe": "await", "ok": true, "elapsed_ms": 12}),
        ),
    ];
    if let Some(slept) = waited_ms {
        results.push(framed(
            "detach-wait",
            1000,
            json!({"probe": "sleep", "ok": true, "slept_ms": slept}),
        ));
    }
    results.extend([
        framed(
            "m4-release-cwd",
            1000,
            cwd_release(
                listing(&[]),
                &[("bin", gone()), ("bin/tool", gone()), ("lib", gone())],
            ),
        ),
        framed("m3-release-fd", 1000, released("fd", "fd", &data())),
        framed("m3-release-map", 1000, released("map", "map", &data())),
        framed("m3-release-exec", 1000, released("exec", "exec", &tool())),
        framed("detach-list-share", 1000, listing(&["opt"])),
        framed("detach-list-opt", 1000, listing(&[])),
        framed("detach-lookup-tree", 1000, gone()),
    ]);
    results
}

fn replace(results: &mut [ProbeResult], id: &str, uid: u32, line: Value) {
    let slot = results
        .iter_mut()
        .find(|result| result.probe == id)
        .expect("the probe is in the fixture");
    *slot = framed(id, uid, line);
}

fn self_check(failing: Option<&str>) -> String {
    let mut text = String::from("lane-vmm: bundle matches its SHA256SUMS\n");
    for check in gate::SELF_CHECKS {
        text.push_str(&format!(
            "lane-vmm: control {check}: expected allowed, observed allowed, holds\n"
        ));
    }
    for (index, check) in gate::SELF_CHECKS.iter().enumerate() {
        let (expected, observed) = if index == 0 {
            ("allowed", "allowed")
        } else {
            ("refused", "EPERM")
        };
        let outcome = if failing == Some(check) {
            "FAILS"
        } else {
            "holds"
        };
        text.push_str(&format!(
            "lane-vmm: self-check {check}: expected {expected}, observed {observed}, {outcome}\n"
        ));
    }
    text
}

#[test]
fn the_detach_probes_hold_four_references_before_the_flag_and_release_after() {
    let ids: Vec<String> = detach_probes(true)
        .into_iter()
        .map(|probe| probe.id)
        .collect();
    let at = |id: &str| ids.iter().position(|probe| probe == id).expect(id);
    assert!(at(gate::DETACH_AFTER) < at("detach-await"));
    assert!(at("detach-await") < at("detach-wait"));
    assert!(at("detach-wait") < at("m4-release-cwd"));
    assert!(at("detach-wait") < at("m3-release-fd"));
    let no_wait: Vec<String> = detach_probes(false)
        .into_iter()
        .map(|probe| probe.id)
        .collect();
    assert!(!no_wait.contains(&"detach-wait".to_string()));
    assert!(detach_probes(true).iter().all(|probe| probe.uid == 1000));
}

#[test]
fn m3_passes_when_every_holder_reads_its_bytes_back_with_the_helper_confined() {
    assert_eq!(
        judge_m3(&detach(Some(5001)), &self_check(None), &data(), &tool()),
        Verdict::Pass
    );
}

#[test]
fn m3_is_not_run_without_a_confined_helper_or_the_hosts_flag() {
    let results = detach(Some(5001));
    assert!(is_not_run(&judge_m3(
        &results,
        "lane-vmm: starting run",
        &data(),
        &tool()
    )));
    assert!(is_not_run(&judge_m3(
        &results,
        &self_check(Some("write outside")),
        &data(),
        &tool()
    )));
    let mut unflagged = detach(Some(5001));
    replace(
        &mut unflagged,
        "detach-await",
        1000,
        json!({"probe": "await", "ok": false, "error": "timed out", "elapsed_ms": 30000}),
    );
    assert!(is_not_run(&judge_m3(
        &unflagged,
        &self_check(None),
        &data(),
        &tool()
    )));
}

#[test]
fn m3_fails_when_a_held_read_fails_or_reads_other_bytes() {
    let check = self_check(None);
    let mut failed = detach(Some(5001));
    replace(
        &mut failed,
        "m3-release-fd",
        1000,
        json!({"probe": "release", "ok": false, "errno": "ENOENT", "name": "fd"}),
    );
    assert!(is_fail(
        &judge_m3(&failed, &check, &data(), &tool()),
        "m3-release-fd"
    ));
    let mut died = detach(Some(5001));
    replace(
        &mut died,
        "m3-release-map",
        1000,
        json!({"probe": "release", "ok": false, "name": "map",
               "error": "the holder closed without a reply"}),
    );
    assert!(is_fail(
        &judge_m3(&died, &check, &data(), &tool()),
        "m3-release-map"
    ));
    let mut other = detach(Some(5001));
    replace(
        &mut other,
        "m3-release-exec",
        1000,
        released("exec", "exec", &expected("cc", 9)),
    );
    assert!(is_fail(
        &judge_m3(&other, &check, &data(), &tool()),
        "m3-release-exec"
    ));
    let mut shorter = detach(Some(5001));
    replace(
        &mut shorter,
        "m3-release-fd",
        1000,
        released("fd", "fd", &expected("aa", 4)),
    );
    assert!(is_fail(
        &judge_m3(&shorter, &check, &data(), &tool()),
        "m3-release-fd"
    ));
}

#[test]
fn m3_fails_when_the_guest_lists_the_holding_directory() {
    let mut results = detach(Some(5001));
    replace(
        &mut results,
        "detach-list-share",
        1000,
        listing(&[gate::HOLDING, "opt"]),
    );
    assert!(is_fail(
        &judge_m3(&results, &self_check(None), &data(), &tool()),
        gate::HOLDING
    ));
}

#[test]
fn m3_is_not_run_when_the_unconfined_control_is_missing_or_fails() {
    let results = detach(Some(5001));
    let no_control: String = self_check(None)
        .lines()
        .filter(|row| !row.contains("control"))
        .map(|row| format!("{row}\n"))
        .collect();
    assert!(!no_control.contains("control"));
    assert!(
        matches!(judge_m3(&results, &no_control, &data(), &tool()), Verdict::NotRun(reason) if reason.contains("control"))
    );
    let failed_control = self_check(None).replace(
        "control read outside: expected allowed, observed allowed, holds",
        "control read outside: expected allowed, observed ENOENT, FAILS",
    );
    assert_ne!(failed_control, self_check(None));
    assert!(
        matches!(judge_m3(&results, &failed_control, &data(), &tool()), Verdict::NotRun(reason) if reason.contains("control read outside"))
    );
}

#[test]
fn m3_fails_when_the_tree_is_still_in_the_share() {
    let mut results = detach(Some(5001));
    replace(&mut results, "detach-list-opt", 1000, listing(&["p"]));

    assert!(is_fail(
        &judge_m3(&results, &self_check(None), &data(), &tool()),
        "opt"
    ));
}

#[test]
fn m3_fails_when_a_holding_directory_is_anywhere_in_the_share() {
    let check = self_check(None);
    let mut in_opt = detach(Some(5001));
    replace(
        &mut in_opt,
        "detach-list-opt",
        1000,
        listing(&[gate::HOLDING]),
    );
    assert!(is_fail(
        &judge_m3(&in_opt, &check, &data(), &tool()),
        gate::HOLDING
    ));
    let mut renamed = detach(Some(5001));
    replace(
        &mut renamed,
        "detach-list-share",
        1000,
        listing(&["kept", "opt"]),
    );
    assert!(is_fail(
        &judge_m3(&renamed, &check, &data(), &tool()),
        "kept"
    ));
    let mut no_names = detach(Some(5001));
    replace(
        &mut no_names,
        "detach-list-opt",
        1000,
        json!({"probe": "list", "ok": true}),
    );
    assert!(is_fail(
        &judge_m3(&no_names, &check, &data(), &tool()),
        "opt"
    ));
    let mut unlisted = detach(Some(5001));
    replace(
        &mut unlisted,
        "detach-list-opt",
        1000,
        refused("list", "EIO"),
    );
    assert!(is_not_run(&judge_m3(&unlisted, &check, &data(), &tool())));
}

#[test]
fn m3_is_not_run_when_a_release_ran_as_root_and_relays_the_holders_uid() {
    let mut results = detach(Some(5001));
    let mut relayed = released("fd", "fd", &data());
    relayed["id_u"] = json!(1000);
    replace(&mut results, "m3-release-fd", 0, relayed);

    assert!(
        matches!(judge_m3(&results, &self_check(None), &data(), &tool()), Verdict::NotRun(reason) if reason.contains("ran as uid 0"))
    );
}

#[test]
fn m3_is_not_run_when_a_release_ran_as_root() {
    let mut results = detach(Some(5001));
    replace(
        &mut results,
        "m3-release-fd",
        0,
        released("fd", "fd", &data()),
    );
    assert!(is_not_run(&judge_m3(
        &results,
        &self_check(None),
        &data(),
        &tool()
    )));
}

#[test]
fn m4_passes_on_an_empty_listing_and_enoent_for_every_former_name_after_the_window() {
    assert_eq!(judge_m4(&detach(Some(5000)), true), Verdict::Pass);
    assert_eq!(judge_m4(&detach(None), false), Verdict::Pass);
}

#[test]
fn m4_is_not_run_when_probed_inside_the_window_or_with_no_former_names() {
    assert!(is_not_run(&judge_m4(&detach(Some(4999)), true)));
    assert!(is_not_run(&judge_m4(&detach(None), true)));
    let mut nothing_held = detach(Some(5000));
    replace(
        &mut nothing_held,
        "m4-release-cwd",
        1000,
        cwd_release(listing(&[]), &[]),
    );
    assert!(is_not_run(&judge_m4(&nothing_held, true)));
}

#[test]
fn m4_fails_on_a_listing_with_any_name_even_when_every_former_name_is_gone() {
    let mut stray = detach(Some(5000));
    replace(
        &mut stray,
        "m4-release-cwd",
        1000,
        cwd_release(listing(&["stray"]), &[("bin", gone())]),
    );

    assert!(is_fail(&judge_m4(&stray, true), "stray"));
}

#[test]
fn m4_fails_on_a_listing_line_with_no_names() {
    let mut malformed = detach(Some(5000));
    replace(
        &mut malformed,
        "m4-release-cwd",
        1000,
        cwd_release(json!({"probe": "list", "ok": true}), &[("bin", gone())]),
    );

    assert!(is_fail(&judge_m4(&malformed, true), "listing"));
}

#[test]
fn m4_fails_when_directories_stay_are_removed_or_a_name_still_resolves() {
    let mut left = detach(Some(5000));
    replace(
        &mut left,
        "m4-release-cwd",
        1000,
        cwd_release(
            listing(&["bin", "lib"]),
            &[("bin", found()), ("lib", found())],
        ),
    );
    assert!(is_fail(&judge_m4(&left, true), "bin"));
    let mut removed = detach(Some(5000));
    replace(
        &mut removed,
        "m4-release-cwd",
        1000,
        cwd_release(refused("list", "ENOENT"), &[("bin", gone())]),
    );
    assert!(is_fail(&judge_m4(&removed, true), "ENOENT"));
    let mut cached = detach(Some(5000));
    replace(
        &mut cached,
        "m4-release-cwd",
        1000,
        cwd_release(listing(&[]), &[("bin", gone()), ("lib", found())]),
    );
    assert!(is_fail(&judge_m4(&cached, true), "lib"));
    let mut tree = detach(Some(5000));
    replace(&mut tree, "detach-lookup-tree", 1000, found());
    assert!(is_fail(&judge_m4(&tree, true), "detach-lookup-tree"));
    let mut eio = detach(Some(5000));
    replace(
        &mut eio,
        "m4-release-cwd",
        1000,
        cwd_release(listing(&[]), &[("bin", refused("lookup", "EIO"))]),
    );
    assert!(is_fail(&judge_m4(&eio, true), "EIO"));
}

fn cases() -> Vec<SourceCase> {
    [
        ("inside the share", true),
        ("around the share", true),
        ("a sibling", false),
    ]
    .into_iter()
    .map(|(name, must_refuse)| SourceCase {
        name: name.to_string(),
        must_refuse,
    })
    .collect()
}

#[test]
fn m5_passes_when_exactly_the_overlapping_sources_are_refused() {
    let answers = vec![Ok(true), Ok(true), Ok(false)];
    assert_eq!(judge_m5(&cases(), &answers), Verdict::Pass);
}

#[test]
fn m5_fails_on_an_accepted_overlap_or_a_refused_sibling_and_is_not_run_on_an_error() {
    assert!(is_fail(
        &judge_m5(&cases(), &[Ok(true), Ok(false), Ok(false)]),
        "around the share"
    ));
    assert!(is_fail(
        &judge_m5(&cases(), &[Ok(true), Ok(true), Ok(true)]),
        "a sibling"
    ));
    assert!(is_not_run(&judge_m5(
        &cases(),
        &[Ok(true), Err(io::Error::other("gone")), Ok(false)]
    )));
    assert!(is_not_run(&judge_m5(&cases(), &[Ok(true)])));
}

#[test]
fn the_reference_check_refuses_sources_inside_or_around_the_share_or_holding() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let root = fs::canonicalize(dir.path()).expect("resolves");
    let share = root.join("software/share");
    let holding = root.join("software/holding");
    for path in [share.join("opt"), holding.join("0"), root.join("elsewhere")] {
        fs::create_dir_all(path).expect("made");
    }
    let refuses = |source: PathBuf| refuses_source(&share, &holding, &source).expect("exists");

    assert!(refuses(share.join("opt")));
    assert!(refuses(root.join("software")));
    assert!(refuses(holding.join("0")));
    assert!(refuses(root.clone()));
    assert!(!refuses(root.join("elsewhere")));
}

fn r1_trial(index: usize, before: Value, after: Value) -> Vec<ProbeResult> {
    let name = format!("w{index}");
    vec![
        framed(
            &gate::r1_watch(index),
            1000,
            json!({"probe": "hold-watch", "ok": true, "name": name, "kind": "watch",
                   "pid": 300, "before": before}),
        ),
        framed(
            &format!("r1-await-{index}"),
            1000,
            json!({"probe": "await", "ok": true, "elapsed_ms": 3}),
        ),
        framed(
            &format!("r1-release-{index}"),
            1000,
            json!({"probe": "release", "ok": true, "name": name, "kind": "watch",
                   "lookup": after}),
        ),
    ]
}

fn file_found() -> Value {
    json!({"probe": "lookup", "ok": true, "kind": "file"})
}

#[test]
fn r1_counts_each_trial_whose_running_process_found_the_new_file() {
    let results: Vec<ProbeResult> = (0..3)
        .flat_map(|index| r1_trial(index, gone(), file_found()))
        .collect();

    let tally = judge_r1(&results, 3);

    assert_eq!(tally.held(), 3);
    assert_eq!(tally.trials.len(), 3);
    assert_eq!(tally.verdict(), Verdict::Pass);
}

#[test]
fn r1_reports_a_trial_that_missed_the_file_or_found_it_early_and_never_rounds_up() {
    let mut results = r1_trial(0, gone(), file_found());
    results.extend(r1_trial(1, gone(), gone()));
    results.extend(r1_trial(2, file_found(), file_found()));

    let tally = judge_r1(&results, 4);

    assert_eq!(tally.held(), 1);
    assert_eq!(tally.trials[0].outcome, Verdict::Pass);
    assert!(matches!(tally.trials[1].outcome, Verdict::Fail(_)));
    assert!(matches!(tally.trials[2].outcome, Verdict::Fail(_)));
    assert!(matches!(tally.trials[3].outcome, Verdict::NotRun(_)));
    assert!(is_fail(&tally.verdict(), "1 of 4"));
}

#[test]
fn r1_fails_a_trial_whose_lookup_after_found_a_directory() {
    let tally = judge_r1(&r1_trial(0, gone(), found()), 1);

    assert!(is_fail(&tally.trials[0].outcome, "dir"));
}

#[test]
fn a_tally_passes_only_on_every_claimed_trial() {
    let trial = |index, outcome| Trial { index, outcome };
    let all = Tally {
        claimed: 2,
        trials: vec![trial(0, Verdict::Pass), trial(1, Verdict::Pass)],
    };
    assert_eq!(all.verdict(), Verdict::Pass);
    let short = Tally {
        claimed: 2,
        trials: vec![trial(0, Verdict::Pass)],
    };
    assert!(matches!(short.verdict(), Verdict::NotRun(reason) if reason.contains("1 of 2")));
    let unrun = Tally {
        claimed: 2,
        trials: vec![
            trial(0, Verdict::Pass),
            trial(1, Verdict::NotRun("no line".to_string())),
        ],
    };
    assert!(matches!(unrun.verdict(), Verdict::NotRun(_)));
    let failed = Tally {
        claimed: 2,
        trials: vec![
            trial(0, Verdict::Fail("missed".to_string())),
            trial(1, Verdict::NotRun("no line".to_string())),
        ],
    };
    assert!(matches!(failed.verdict(), Verdict::Fail(reason) if reason.contains("0 of 2")));
}

fn stage_with(
    config: &str,
    mount: Value,
    initial_stat: Value,
    workload_stat: Value,
) -> ProbeResult {
    framed(
        &format!("r2-{config}"),
        0,
        json!({"probe": "stage", "ok": true, "config": config, "mount": mount,
               "initial_stat": initial_stat, "workload_stat": workload_stat}),
    )
}

fn stage(config: &str, mount: Value, workload_stat: Value) -> ProbeResult {
    stage_with(
        config,
        mount,
        json!({"probe": "stat", "ok": true, "kind": "file", "id_u": 0}),
        workload_stat,
    )
}

fn workload_stat(line: Value) -> Value {
    let mut line = line;
    line["id_u"] = json!(1000);
    line
}

fn r2(private: Value, unreachable: Value) -> Vec<ProbeResult> {
    vec![
        stage(
            "visible",
            json!({"ok": true}),
            workload_stat(json!({"probe": "stat", "ok": true, "kind": "file"})),
        ),
        stage("private", json!({"ok": true}), workload_stat(private)),
        stage(
            "unreachable",
            json!({"ok": true}),
            workload_stat(unreachable),
        ),
        stage(
            "private-parent",
            json!({"ok": true}),
            workload_stat(refused("stat", "ENOENT")),
        ),
        stage(
            "late-directory",
            json!({"ok": true}),
            workload_stat(refused("stat", "ENOENT")),
        ),
    ]
}

#[test]
fn r2_shows_a_successful_mount_the_workload_cannot_see() {
    let verdicts = judge_r2(&r2(refused("stat", "ENOENT"), refused("stat", "ENOENT")));

    assert_eq!(
        verdicts,
        vec![
            ("visible".to_string(), Verdict::Pass),
            ("private".to_string(), Verdict::Pass),
            ("unreachable".to_string(), Verdict::Pass),
            ("private-parent".to_string(), Verdict::Pass),
            ("late-directory".to_string(), Verdict::Pass),
        ]
    );
}

#[test]
fn r2_fails_a_parent_private_configuration_the_workload_could_see() {
    for index in [3, 4] {
        let mut results = r2(refused("stat", "ENOENT"), refused("stat", "ENOENT"));
        let config = gate::R2_CONFIGS[index];
        results[index] = stage(
            config,
            json!({"ok": true}),
            workload_stat(json!({"probe": "stat", "ok": true, "kind": "file"})),
        );

        assert!(is_fail(&judge_r2(&results)[index].1, config), "{config}");
    }
}

#[test]
fn r2_is_not_run_in_any_configuration_whose_staging_left_no_file() {
    for (index, config) in gate::R2_CONFIGS.iter().enumerate() {
        let seen = if *config == "visible" {
            json!({"probe": "stat", "ok": true, "kind": "file"})
        } else {
            refused("stat", "ENOENT")
        };
        let mut results = r2(refused("stat", "ENOENT"), refused("stat", "ENOENT"));
        results[index] = stage_with(
            config,
            json!({"ok": true}),
            json!({"probe": "stat", "ok": false, "errno": "ENOENT", "id_u": 0}),
            workload_stat(seen.clone()),
        );
        assert!(
            matches!(&judge_r2(&results)[index].1, Verdict::NotRun(reason) if reason.contains("initial")),
            "{config}"
        );
        results[index] = stage_with(
            config,
            json!({"ok": true}),
            json!({"probe": "stat", "ok": true, "kind": "dir", "id_u": 0}),
            workload_stat(seen),
        );
        assert!(is_not_run(&judge_r2(&results)[index].1), "{config}");
    }
}

#[test]
fn r2_fails_the_visible_control_when_the_workload_finds_a_directory() {
    let mut results = r2(refused("stat", "ENOENT"), refused("stat", "ENOENT"));
    results[0] = stage(
        "visible",
        json!({"ok": true}),
        workload_stat(json!({"probe": "stat", "ok": true, "kind": "dir"})),
    );

    assert!(is_fail(&judge_r2(&results)[0].1, "visible"));
}

#[test]
fn r2_fails_a_configuration_the_workload_could_see_and_skips_a_failed_mount() {
    let mut results = r2(
        json!({"probe": "stat", "ok": true, "kind": "file"}),
        refused("stat", "ENOENT"),
    );
    results[2] = stage(
        "unreachable",
        json!({"ok": false, "errno": "EINVAL"}),
        workload_stat(refused("stat", "ENOENT")),
    );

    let verdicts = judge_r2(&results);

    assert!(is_fail(&verdicts[1].1, "private"));
    assert!(is_not_run(&verdicts[2].1));
}

#[test]
fn r2_fails_the_visible_control_when_the_workload_cannot_see_the_file() {
    let mut results = r2(refused("stat", "ENOENT"), refused("stat", "ENOENT"));
    results[0] = stage(
        "visible",
        json!({"ok": true}),
        workload_stat(refused("stat", "ENOENT")),
    );

    assert!(is_fail(&judge_r2(&results)[0].1, "visible"));
}

#[test]
fn r2_is_not_run_when_the_workload_stat_ran_as_root() {
    let mut results = r2(refused("stat", "ENOENT"), refused("stat", "ENOENT"));
    results[1] = stage(
        "private",
        json!({"ok": true}),
        json!({"probe": "stat", "ok": false, "errno": "ENOENT", "id_u": 0}),
    );

    assert!(is_not_run(&judge_r2(&results)[1].1));
}

fn r3_trial(index: usize, lookup: Value, fd: Value) -> Vec<ProbeResult> {
    let ok = |probe: &str| json!({"probe": probe, "ok": true});
    vec![
        framed(&format!("r3-mount-{index}"), 0, ok("mount")),
        framed(&format!("r3-hold-fd-{index}"), 1000, ready("hold-fd", "fd")),
        framed(
            &format!("r3-hold-map-{index}"),
            1000,
            ready("hold-map", "map"),
        ),
        framed(&format!("r3-detach-{index}"), 0, ok("umount-detach")),
        framed(&format!("r3-lookup-{index}"), 1000, lookup),
        framed(&format!("r3-release-fd-{index}"), 1000, fd),
        framed(
            &format!("r3-release-map-{index}"),
            1000,
            released("map", "map", &data()),
        ),
    ]
}

#[test]
fn r3_counts_trials_whose_holders_read_their_bytes_after_the_lazy_unmount() {
    let mut results = r3_trial(0, gone(), released("fd", "fd", &data()));
    results.extend(r3_trial(1, file_found(), released("fd", "fd", &data())));
    results.extend(r3_trial(
        2,
        gone(),
        released("fd", "fd", &expected("zz", 5)),
    ));

    let tally = judge_r3(&results, 3, &data());

    assert_eq!(tally.trials[0].outcome, Verdict::Pass);
    assert!(matches!(tally.trials[1].outcome, Verdict::Fail(_)));
    assert!(matches!(tally.trials[2].outcome, Verdict::Fail(_)));
    assert_eq!(tally.held(), 1);
}

#[test]
fn r3_is_not_run_for_a_trial_whose_mount_or_unmount_failed() {
    let mut results = r3_trial(0, gone(), released("fd", "fd", &data()));
    results[0] = framed("r3-mount-0", 0, refused("mount", "EBUSY"));
    results.extend(r3_trial(1, gone(), released("fd", "fd", &data())));
    results[10] = framed("r3-detach-1", 0, refused("umount-detach", "EINVAL"));

    let tally = judge_r3(&results, 2, &data());

    assert!(matches!(tally.trials[0].outcome, Verdict::NotRun(_)));
    assert!(matches!(tally.trials[1].outcome, Verdict::NotRun(_)));
}

fn hit(pid: u64, tid: Option<u64>, view: &str, entry: &str) -> Value {
    json!({"pid": pid, "tid": tid, "view": view, "entry": entry, "target": "x"})
}

fn r4_thread(index: usize, hits: Vec<Value>, release: Value) -> Vec<ProbeResult> {
    vec![
        framed(
            &format!("r4a-hold-{index}"),
            1000,
            json!({"probe": "hold-thread", "ok": true, "name": format!("t{index}"),
                   "kind": "thread", "pid": 400, "tid": 401}),
        ),
        framed(
            &format!("r4a-scan-{index}"),
            0,
            json!({"probe": "scan", "ok": true, "targets": [], "hits": hits}),
        ),
        framed(&format!("r4a-release-{index}"), 1000, release),
    ]
}

#[test]
fn r4_finds_the_threads_own_table_only_in_its_task_entry() {
    let task_only = vec![
        hit(400, Some(401), "task", "fd/3"),
        hit(400, Some(401), "task", "cwd"),
    ];
    let mut results = r4_thread(0, task_only, released("t0", "thread", &data()));
    results.extend(r4_thread(
        1,
        vec![
            hit(400, None, "process", "fd/3"),
            hit(400, Some(401), "task", "fd/3"),
            hit(400, Some(401), "task", "cwd"),
        ],
        released("t1", "thread", &data()),
    ));
    results.extend(r4_thread(2, vec![], released("t2", "thread", &data())));
    results.extend(r4_thread(
        3,
        vec![
            hit(400, Some(401), "task", "fd/3"),
            hit(400, Some(401), "task", "cwd"),
        ],
        released("t3", "thread", &expected("zz", 5)),
    ));

    let tally = judge_r4_thread(&results, 4, &data());

    assert_eq!(tally.trials[0].outcome, Verdict::Pass);
    assert!(
        matches!(&tally.trials[1].outcome, Verdict::Fail(reason) if reason.contains("process"))
    );
    assert!(matches!(tally.trials[2].outcome, Verdict::Fail(_)));
    assert!(matches!(tally.trials[3].outcome, Verdict::Fail(_)));
}

fn r4_in_flight(index: usize, hits: Vec<Value>, release: Value) -> Vec<ProbeResult> {
    vec![
        framed(
            &format!("r4b-hold-{index}"),
            1000,
            json!({"probe": "hold-inflight", "ok": true, "name": format!("f{index}"),
                   "kind": "inflight", "pid": 500}),
        ),
        framed(
            &format!("r4b-scan-{index}"),
            0,
            json!({"probe": "scan", "ok": true, "targets": [], "hits": hits}),
        ),
        framed(&format!("r4b-release-{index}"), 1000, release),
    ]
}

#[test]
fn r4_finds_no_table_holding_a_descriptor_in_flight() {
    let mut results = r4_in_flight(0, vec![], released("f0", "inflight", &data()));
    results.extend(r4_in_flight(
        1,
        vec![hit(500, None, "process", "fd/4")],
        released("f1", "inflight", &data()),
    ));
    results.extend(r4_in_flight(
        2,
        vec![],
        json!({"probe": "release", "ok": false, "errno": "EBADF", "name": "f2"}),
    ));

    let tally = judge_r4_in_flight(&results, 3, &data());

    assert_eq!(tally.trials[0].outcome, Verdict::Pass);
    assert!(matches!(tally.trials[1].outcome, Verdict::Fail(_)));
    assert!(matches!(tally.trials[2].outcome, Verdict::Fail(_)));
}

#[test]
fn the_name_observation_says_one_name_when_both_spellings_reach_one_inode() {
    let file = |ino: u64| {
        json!({"probe": "stat", "ok": true, "kind": "file", "uid": 0, "gid": 0,
               "mode": "0644", "size": 1, "nlink": 1, "dev": 23, "ino": ino})
    };
    let results = vec![
        framed("name-0-placed", 1000, file(10)),
        framed("name-0-other", 1000, file(10)),
        framed("name-0-list", 1000, listing(&["a"])),
        framed("name-1-placed", 1000, file(11)),
        framed("name-1-other", 1000, refused("stat", "ENOENT")),
        framed("name-1-list", 1000, listing(&["b"])),
        framed("name-2-placed", 1000, file(12)),
        framed("name-2-other", 1000, file(13)),
        framed("name-2-list", 1000, listing(&["c", "d"])),
    ];

    let seen = observe_names(&results);

    assert_eq!(seen[0].1, "one name");
    assert_eq!(seen[1].1, "two names");
    assert!(seen[2].1.contains("13"), "{}", seen[2].1);
    assert_eq!(seen[0].0, gate::NAME_PAIRS[0].0);
    assert_eq!(seen.len(), gate::NAME_PAIRS.len());
    assert!(seen[3].1.starts_with("not run"), "{}", seen[3].1);
}

#[test]
fn the_name_pairs_are_the_eight_measured_on_apfs() {
    let codes = |text: &str| text.chars().map(|c| c as u32).collect::<Vec<_>>();
    assert_eq!(codes(gate::NAME_PAIRS[0].1), [0x63, 0x61, 0x66, 0xe9]);
    assert_eq!(
        codes(gate::NAME_PAIRS[0].2),
        [0x63, 0x61, 0x66, 0x65, 0x301]
    );
    assert_eq!(codes(gate::NAME_PAIRS[1].1), [0x3c3, 0x3b1, 0x3c2]);
    assert_eq!(codes(gate::NAME_PAIRS[1].2), [0x3c3, 0x3b1, 0x3c3]);
    assert_eq!(codes(gate::NAME_PAIRS[3].1), [0xfb01, 0x6c, 0x65]);
    assert_eq!(
        codes(gate::NAME_PAIRS[4].2),
        [0x43, 0x41, 0x46, 0x45, 0x301]
    );
    assert_eq!(codes(gate::NAME_PAIRS[5].1), [0x212a]);
    assert_eq!(codes(gate::NAME_PAIRS[6].2), [0x69, 0x307]);
    assert_eq!(codes(gate::NAME_PAIRS[7].1), [0x1e9e]);
    assert_eq!(codes(gate::NAME_PAIRS[7].2), [0xdf]);
}

fn ids_and_uids(probes: &[plasmosome_lanes::config::Probe]) -> Vec<(String, u32)> {
    probes
        .iter()
        .map(|probe| (probe.id.clone(), probe.uid))
        .collect()
}

fn owned(pairs: &[(&str, u32)]) -> Vec<(String, u32)> {
    pairs
        .iter()
        .map(|(id, uid)| (id.to_string(), *uid))
        .collect()
}

#[test]
fn the_share_probes_are_the_ones_m1_m2_and_the_setuid_observation_read() {
    assert_eq!(
        ids_and_uids(&gate::m1_probes()),
        owned(&[("m1-share", 1000), ("m1-file", 1000)])
    );
    assert_eq!(
        ids_and_uids(&gate::m2_probes()),
        owned(&[
            ("m2-workload-create", 1000),
            ("m2-root-create", 0),
            ("m2-mountinfo", 1000)
        ])
    );
    assert_eq!(
        gate::m1_probes()[1].args,
        ["stat", "/mnt/sw/opt/p/bin/tool"]
    );
    assert_eq!(
        ids_and_uids(&gate::suid_probes()),
        owned(&[("suid-hold", 1000), ("suid-release", 1000)])
    );
    assert_eq!(
        gate::suid_probes()[0].args,
        ["hold-exec", "suid", "/mnt/sw/opt/suid/tool"]
    );
}

#[test]
fn the_r1_probes_watch_await_and_release_each_trial_in_turn() {
    let probes = gate::r1_probes(2);

    assert_eq!(
        ids_and_uids(&probes),
        owned(&[
            ("r1-watch-0", 1000),
            ("r1-await-0", 1000),
            ("r1-release-0", 1000),
            ("r1-watch-1", 1000),
            ("r1-await-1", 1000),
            ("r1-release-1", 1000),
        ])
    );
    assert_eq!(
        probes[3].args,
        ["hold-watch", "w1", "/mnt/sw/opt/r1-1/bin/x"]
    );
    assert_eq!(probes[4].args, ["await", "/mnt/ctl/r1-1", "30000"]);
    assert_eq!(probes[5].args, ["release", "w1"]);
    assert_eq!(gate::r1_path(1), "opt/r1-1/bin/x");
    assert_eq!(gate::r1_flag(1), "r1-1");
}

#[test]
fn the_r2_r3_and_r4_probes_run_as_the_uids_their_judges_read() {
    assert_eq!(
        ids_and_uids(&gate::r2_probes()),
        owned(&[
            ("r2-visible", 0),
            ("r2-private", 0),
            ("r2-unreachable", 0),
            ("r2-private-parent", 0),
            ("r2-late-directory", 0),
        ])
    );
    assert_eq!(gate::r2_probes()[1].args, ["stage", "private"]);
    assert_eq!(gate::r2_probes()[4].args, ["stage", "late-directory"]);
    let r3 = gate::r3_probes(1);
    assert_eq!(
        ids_and_uids(&r3),
        owned(&[
            ("r3-mount-0", 0),
            ("r3-hold-fd-0", 1000),
            ("r3-hold-map-0", 1000),
            ("r3-detach-0", 0),
            ("r3-lookup-0", 1000),
            ("r3-release-fd-0", 1000),
            ("r3-release-map-0", 1000),
        ])
    );
    assert_eq!(r3[0].args, ["mount", "r3", "/mnt/r3/0", "virtiofs"]);
    assert_eq!(r3[3].args, ["umount-detach", "/mnt/r3/0"]);
    let r4 = gate::r4_probes(1);
    assert_eq!(
        ids_and_uids(&r4),
        owned(&[
            ("r4a-hold-0", 1000),
            ("r4a-scan-0", 0),
            ("r4a-release-0", 1000),
            ("r4b-hold-0", 1000),
            ("r4b-scan-0", 0),
            ("r4b-release-0", 1000),
        ])
    );
    assert_eq!(
        r4[0].args,
        [
            "hold-thread",
            "t0",
            "/mnt/sw/opt/r4/data.txt",
            "/mnt/sw/opt/r4/dir"
        ]
    );
    assert_eq!(r4[4].args, ["scan", "/mnt/sw/opt/r4/data.txt"]);
}

#[test]
fn the_name_probes_stat_both_spellings_of_each_pair_and_list_its_directory() {
    let probes = gate::name_probes();

    assert_eq!(probes.len(), 3 * gate::NAME_PAIRS.len());
    let (_, placed, other) = gate::NAME_PAIRS[2];
    assert_eq!(probes[6].id, "name-2-placed");
    assert_eq!(
        probes[6].args,
        ["stat".to_string(), format!("/mnt/sw/names/2/{placed}")]
    );
    assert_eq!(
        probes[7].args,
        ["stat".to_string(), format!("/mnt/sw/names/2/{other}")]
    );
    assert_eq!(probes[8].args, ["list", "/mnt/sw/names/2"]);
    assert_eq!(gate::name_dir(2), "names/2");
}

#[test]
fn result_of_finds_a_probes_line_or_nothing() {
    let results = vec![
        framed("a", 1000, json!({"probe": "id", "ok": true})),
        ProbeResult {
            probe: "b".to_string(),
            uid: 1000,
            exit: None,
            result: None,
        },
    ];

    assert_eq!(
        gate::result_of(&results, "a").map(|line| &line["ok"]),
        Some(&json!(true))
    );
    assert_eq!(gate::result_of(&results, "b"), None);
    assert_eq!(gate::result_of(&results, "c"), None);
}

#[test]
fn a_probe_that_printed_no_result_line_is_not_run() {
    let mut results = m1((0, 0));
    results[0].result = None;

    assert!(
        matches!(judge_m1(&results), Verdict::NotRun(reason) if reason.contains("no result line"))
    );
}

#[test]
fn r1_is_not_run_when_the_first_lookup_failed_other_than_enoent() {
    let tally = judge_r1(&r1_trial(0, refused("lookup", "EACCES"), file_found()), 1);

    assert!(is_not_run(&tally.trials[0].outcome));
}

#[test]
fn r4_fails_a_thread_with_no_working_directory_and_is_not_run_with_no_hit_list() {
    let no_cwd = r4_thread(
        0,
        vec![hit(400, Some(401), "task", "fd/3")],
        released("t0", "thread", &data()),
    );
    assert!(is_fail(
        &judge_r4_thread(&no_cwd, 1, &data()).trials[0].outcome,
        "working directory"
    ));
    let mut no_list = r4_thread(0, vec![], released("t0", "thread", &data()));
    replace(
        &mut no_list,
        "r4a-scan-0",
        0,
        json!({"probe": "scan", "ok": true}),
    );
    assert!(is_not_run(
        &judge_r4_thread(&no_list, 1, &data()).trials[0].outcome
    ));
}

#[test]
fn the_name_observation_reports_another_error_as_seen() {
    let mut results = Vec::new();
    for index in 0..gate::NAME_PAIRS.len() {
        results.push(framed(
            &format!("name-{index}-placed"),
            1000,
            stat(1000, 1000),
        ));
        results.push(framed(
            &format!("name-{index}-other"),
            1000,
            refused("stat", "EACCES"),
        ));
    }

    let seen = observe_names(&results);

    assert_eq!(seen[0].1, "the other spelling gave EACCES");
}
