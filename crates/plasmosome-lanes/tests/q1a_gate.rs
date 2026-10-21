#![cfg(target_os = "macos")]

mod common;

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::gate::{
    GateLane, Shown, assert_caught, assert_decided, commit_at, confine, gate_lane, own_as_root,
    place_tree, read_records, record_lane, records_dir, results, run_gate, set_override, spelled,
    test_commit, walk, write_file, write_record,
};
use common::guest::{TIMEOUT, guest_binary, probe};
use plasmosome_lanes::config::{MountOption, Semantics, WORKLOAD_UID};
use plasmosome_lanes::console::ProbeResult;
use plasmosome_lanes::gate::{
    self, DETACH_AFTER, DETACH_FLAG, Expected, HOLDING, NAME_PAIRS, SHARE_MOUNT, SUID_TOOL,
    SourceCase, TREE, Verdict,
};
use plasmosome_lanes::holding::move_to_holding;
use plasmosome_lanes::results::{Expect, Record, RowSpec, RunSpec, one_commit, render};
use plasmosome_lanes::steps::{Action, Step};
use serde_json::json;

const RESULTS_FILE: &str = "2026-10-20-spec027-q1a-gate.md";

struct ShareRun {
    name: &'static str,
    semantics: Semantics,
    read_only: bool,
    options: Vec<MountOption>,
    owned: bool,
    names: bool,
    suid: bool,
}

impl ShareRun {
    fn new(name: &'static str, owned: bool) -> ShareRun {
        ShareRun {
            name,
            semantics: Semantics::Complete,
            read_only: true,
            options: Vec::new(),
            owned,
            names: false,
            suid: false,
        }
    }
}

fn place_names(share: &Path) {
    for (index, (_, placed, _)) in NAME_PAIRS.iter().enumerate() {
        write_file(
            &share.join(gate::name_dir(index)).join(placed),
            format!("pair {index}\n").as_bytes(),
            0o644,
        );
    }
}

fn host_exclusive_create(share: &Path) -> Vec<String> {
    NAME_PAIRS
        .iter()
        .enumerate()
        .map(|(index, (_, _, other))| {
            let path = share.join(gate::name_dir(index)).join(other);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => "created a second file".to_string(),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => "EEXIST".to_string(),
                Err(error) => format!("{error}"),
            }
        })
        .collect()
}

fn names_seen(results: &[ProbeResult], share: &Path) -> Vec<String> {
    let host = host_exclusive_create(share);
    gate::observe_names(results)
        .into_iter()
        .zip(NAME_PAIRS)
        .zip(host)
        .map(|(((label, seen), (_, placed, other)), host)| {
            format!(
                "{label} (`{}` placed, `{}` looked up): the guest saw {seen}; on the host an exclusive create of the second spelling gave {host}",
                spelled(placed),
                spelled(other)
            )
        })
        .collect()
}

fn suid_seen(results: &[ProbeResult], options: &[MountOption]) -> Vec<String> {
    let field = |id: &str, key: &str| {
        gate::result_of(results, id)
            .map_or_else(|| "no result".to_string(), |line| line[key].to_string())
    };
    vec![format!(
        "guest mount options {}: the workload's stat of `{SUID_TOOL}` gave mode {} owned by {}:{}; the copy it ran started with effective uid {}, and its release ran as effective uid {}",
        serde_json::to_string(options).unwrap_or_default(),
        field("suid-stat", "mode"),
        field("suid-stat", "uid"),
        field("suid-stat", "gid"),
        field("suid-hold", "id_u"),
        field("suid-release", "id_u"),
    )]
}

fn share_lane(spec: &ShareRun) -> (GateLane, Vec<String>) {
    let mut probes = gate::m1_probes();
    probes.extend(gate::m2_probes());
    if spec.names {
        probes.extend(gate::name_probes());
    }
    if spec.suid {
        probes.push(probe(
            "suid-stat",
            WORKLOAD_UID,
            &["stat", &format!("{SHARE_MOUNT}/{SUID_TOOL}")],
        ));
        probes.extend(gate::suid_probes());
    }
    let gate = gate_lane(
        spec.name,
        probes,
        spec.semantics,
        spec.read_only,
        spec.options.clone(),
    );
    place_tree(&gate.share);
    let owner = fs::metadata(&gate.share).expect("the share exists");
    let mut layout = vec![format!(
        "share: {TREE} with bin/tool (a copy of lane-probe, 0755), lib/data.txt and share/doc/readme.txt (0644); host owner {}:{}",
        owner.uid(),
        owner.gid()
    )];
    if spec.names {
        place_names(&gate.share);
        layout.push("share: names/<i>/<the first spelling of pair i> (0644)".to_string());
    }
    if spec.suid {
        write_file(
            &gate.share.join(SUID_TOOL),
            &guest_binary("lane-probe"),
            0o755,
        );
        layout.push(format!("share: {SUID_TOOL}, a copy of lane-probe (0755)"));
    }
    if spec.owned {
        own_as_root(&gate.share);
        layout.push(
            "every share entry carries user.containers.override_stat 0:0:<its own mode>"
                .to_string(),
        );
        if spec.suid {
            set_override(&[gate.share.join(SUID_TOOL)], "0:0:4755");
            layout.push(format!(
                "{SUID_TOOL} carries user.containers.override_stat 0:0:4755"
            ));
        }
    } else {
        layout.push("no entry carries an override attribute".to_string());
    }
    (gate, layout)
}

fn m_rows(owned: bool) -> (&'static str, &'static str) {
    if owned {
        ("M1 (override_stat)", "M2 (override_stat)")
    } else {
        ("M1", "M2")
    }
}

fn run_share(spec: ShareRun) -> (Verdict, Verdict) {
    let (gate, layout) = share_lane(&spec);
    let run = run_gate(&gate, Vec::new(), TIMEOUT);
    let results = results(&run.run);
    let m1 = gate::judge_m1(results);
    let m2 = gate::judge_m2(results);
    let (m1_row, m2_row) = m_rows(spec.owned);
    let mut shown = Shown::default();
    shown.verdicts.insert(m1_row.to_string(), m1.clone());
    shown.verdicts.insert(m2_row.to_string(), m2.clone());
    if spec.names {
        shown
            .observations
            .insert("Names".to_string(), names_seen(results, &gate.share));
    }
    if spec.suid {
        shown
            .observations
            .insert("Setuid".to_string(), suid_seen(results, &spec.options));
    }
    let layout: Vec<&str> = layout.iter().map(String::as_str).collect();
    record_lane(spec.name, &gate, &layout, &run, shown);
    (m1, m2)
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m1_and_m2_on_a_read_only_complete_share_as_planned() {
    let (m1, m2) = run_share(ShareRun {
        names: true,
        ..ShareRun::new("share-plain", false)
    });

    assert_decided("M1", &m1);
    assert_decided("M2", &m2);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m1_catches_a_share_served_simplified() {
    let (m1, _) = run_share(ShareRun {
        semantics: Semantics::Simplified,
        ..ShareRun::new("share-plain-simplified", false)
    });

    assert_caught("M1", &m1);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m2_catches_a_share_mounted_ro() {
    let (_, m2) = run_share(ShareRun {
        options: vec![MountOption::Ro],
        ..ShareRun::new("share-plain-ro", false)
    });

    assert_caught("M2", &m2);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m2_catches_a_share_added_writable() {
    let (_, m2) = run_share(ShareRun {
        read_only: false,
        ..ShareRun::new("share-plain-writable", false)
    });

    assert_caught("M2", &m2);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m1_and_m2_on_a_share_whose_entries_override_their_owner_to_root() {
    let (m1, m2) = run_share(ShareRun {
        suid: true,
        ..ShareRun::new("share-owned", true)
    });

    assert_decided("M1 (override_stat)", &m1);
    assert_decided("M2 (override_stat)", &m2);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m1_with_override_stat_catches_a_share_served_simplified() {
    let (m1, _) = run_share(ShareRun {
        semantics: Semantics::Simplified,
        ..ShareRun::new("share-owned-simplified", true)
    });

    assert_caught("M1 (override_stat)", &m1);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m2_with_override_stat_catches_a_share_mounted_ro() {
    let (_, m2) = run_share(ShareRun {
        options: vec![MountOption::Ro],
        ..ShareRun::new("share-owned-ro", true)
    });

    assert_caught("M2 (override_stat)", &m2);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m2_with_override_stat_catches_a_share_added_writable() {
    let (_, m2) = run_share(ShareRun {
        read_only: false,
        ..ShareRun::new("share-owned-writable", true)
    });

    assert_caught("M2 (override_stat)", &m2);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn the_setuid_bit_override_stat_reports_with_the_share_mounted_nosuid() {
    let options = vec![MountOption::Nosuid];
    let (gate, layout) = share_lane(&ShareRun {
        options: options.clone(),
        suid: true,
        ..ShareRun::new("share-owned-nosuid", true)
    });
    let run = run_gate(&gate, Vec::new(), TIMEOUT);
    let seen = suid_seen(results(&run.run), &options);
    let mut shown = Shown::default();
    shown.observations.insert("Setuid".to_string(), seen);
    let layout: Vec<&str> = layout.iter().map(String::as_str).collect();
    let record = record_lane("share-owned-nosuid", &gate, &layout, &run, shown);

    assert!(
        gate::result_of(results(&run.run), "suid-hold").is_some(),
        "{record:?}"
    );
}

#[derive(Clone, Copy, Debug)]
enum Host {
    Move,
    Delete,
    LeaveDirs,
    RemoveDirs,
}

impl Host {
    fn describe(self) -> &'static str {
        match self {
            Host::Move => {
                "the host moves every entry of the tree to the holding directory, deepest first, directories included"
            }
            Host::Delete => "the host deletes the tree",
            Host::LeaveDirs => {
                "the host moves every file of the tree to the holding directory and leaves its directories"
            }
            Host::RemoveDirs => {
                "the host moves every file of the tree to the holding directory, then removes its directories"
            }
        }
    }
}

fn move_files(tree: &Path, holding: &Path) -> io::Result<usize> {
    let files: Vec<PathBuf> = walk(tree)
        .into_iter()
        .rev()
        .filter(|path| fs::symlink_metadata(path).is_ok_and(|found| !found.is_dir()))
        .collect();
    for file in &files {
        move_to_holding(file, holding)?;
    }
    Ok(files.len())
}

fn detach_action(host: Host, tree: PathBuf, holding: PathBuf) -> Action {
    Box::new(move || match host {
        Host::Move => move_to_holding(&tree, &holding)
            .map(|moved| format!("moved {} entries, deepest first", moved.len())),
        Host::Delete => fs::remove_dir_all(&tree).map(|()| "deleted the tree".to_string()),
        Host::LeaveDirs => move_files(&tree, &holding)
            .map(|count| format!("moved {count} files and left the directories")),
        Host::RemoveDirs => {
            let count = move_files(&tree, &holding)?;
            fs::remove_dir_all(&tree)?;
            Ok(format!("moved {count} files and removed the directories"))
        }
    })
}

fn holders_seen(results: &[ProbeResult], data: &Expected, tool: &Expected) -> Vec<String> {
    [
        ("m3-release-fd", "the descriptor", data),
        ("m3-release-map", "the mapping", data),
        ("m3-release-exec", "the running binary", tool),
    ]
    .into_iter()
    .map(|(id, what, expected)| {
        let seen = match gate::result_of(results, id) {
            None => "left no result line".to_string(),
            Some(line)
                if line["ok"] == true
                    && line["sha256"] == expected.sha256.as_str()
                    && line["bytes"] == expected.bytes =>
            {
                "read its bytes back".to_string()
            }
            Some(line) if line["ok"] == true => {
                format!("read {} bytes that are not the ones placed", line["bytes"])
            }
            Some(line) => format!(
                "failed: {}",
                line["errno"]
                    .as_str()
                    .or_else(|| line["error"].as_str())
                    .unwrap_or("an unnamed error")
            ),
        };
        format!("`{id}`, {what}: {seen}")
    })
    .collect()
}

fn run_detach(name: &'static str, host: Host, wait: bool) -> (Verdict, Verdict, Record) {
    let mut gate = gate_lane(
        name,
        gate::detach_probes(wait),
        Semantics::Complete,
        true,
        Vec::new(),
    );
    let (data, tool) = place_tree(&gate.share);
    confine(&mut gate);
    let steps = vec![Step {
        after: DETACH_AFTER.to_string(),
        flag: gate.ctl.join(DETACH_FLAG),
        action: detach_action(host, gate.share.join(TREE), gate.holding.clone()),
    }];
    let run = run_gate(&gate, steps, TIMEOUT);
    let results = results(&run.run);
    let m3 = gate::judge_m3(results, &run.run.stderr, &data, &tool, &run.steps);
    let m4 = gate::judge_m4(results, wait);
    let mut shown = Shown::default();
    if wait {
        shown.verdicts.insert("M3".to_string(), m3.clone());
        shown.verdicts.insert("M4".to_string(), m4.clone());
        shown.observations.insert(
            "M3 holders".to_string(),
            holders_seen(results, &data, &tool),
        );
    } else {
        shown.observations.insert(
            "Cache window".to_string(),
            vec![format!(
                "with no wait after the host's flag, M3 gave {} and M4 gave {}",
                verdict_text(&m3),
                verdict_text(&m4)
            )],
        );
    }
    let tree = format!(
        "share: {TREE} with bin/tool (a copy of lane-probe, 0755), lib/data.txt and share/doc/readme.txt (0644); no override attribute"
    );
    let holding = format!("holding directory: software/{HOLDING}, beside the share and outside it");
    let wait_text = if wait {
        "the guest waits 5000 ms after the host's flag before it releases the holders"
    } else {
        "the guest releases the holders as soon as the host's flag arrives"
    };
    let record = record_lane(
        name,
        &gate,
        &[&tree, &holding, host.describe(), wait_text],
        &run,
        shown,
    );
    (m3, m4, record)
}

fn verdict_text(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Pass => "pass".to_string(),
        Verdict::Fail(why) => format!("fail ({why})"),
        Verdict::NotRun(why) => format!("not run ({why})"),
    }
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m3_catches_a_host_that_deletes_the_tree() {
    let (m3, _, _) = run_detach("detach-delete", Host::Delete, true);

    assert_caught("M3", &m3);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m4_catches_a_host_that_leaves_the_directories() {
    let (_, m4, _) = run_detach("detach-leave-dirs", Host::LeaveDirs, true);

    assert_caught("M4", &m4);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m4_catches_a_host_that_removes_the_directories() {
    let (_, m4, _) = run_detach("detach-remove-dirs", Host::RemoveDirs, true);

    assert_caught("M4", &m4);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m3_and_m4_on_a_tree_moved_to_holding_with_lane_vmm_confined() {
    let (m3, m4, _) = run_detach("detach-move", Host::Move, true);

    assert_decided("M3", &m3);
    assert_decided("M4", &m4);
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn m4_with_no_wait_shows_whether_the_cache_window_exists() {
    let (_, _, record) = run_detach("detach-nowait", Host::Move, false);

    assert!(!record.lines.is_empty(), "{record:?}");
}

type Check = fn(&Path, &Path, &Path) -> io::Result<bool>;

fn identity(path: &Path) -> io::Result<(u64, u64)> {
    let found = fs::metadata(path)?;
    Ok((found.dev(), found.ino()))
}

fn within(inner: &Path, outer: &Path) -> io::Result<bool> {
    let outer = identity(outer)?;
    for ancestor in fs::canonicalize(inner)?.ancestors() {
        if identity(ancestor)? == outer {
            return Ok(true);
        }
    }
    Ok(false)
}

fn accepts_a_source_inside_the_share(
    share: &Path,
    holding: &Path,
    source: &Path,
) -> io::Result<bool> {
    Ok(within(share, source)? || within(holding, source)?)
}

fn accepts_a_share_inside_a_source(
    share: &Path,
    holding: &Path,
    source: &Path,
) -> io::Result<bool> {
    Ok(within(source, share)? || within(source, holding)?)
}

fn run_m5(name: &str, what: &str, check: Check) -> Verdict {
    let dir = tempfile::tempdir().expect("a directory for M5");
    let root = fs::canonicalize(dir.path()).expect("the directory resolves");
    let software = root.join("software");
    let share = software.join("share");
    let holding = software.join(HOLDING);
    let inside_share = share.join(TREE);
    let inside_holding = holding.join("000000");
    for path in [
        &inside_share,
        &inside_holding,
        &software.join("sibling"),
        &root.join("elsewhere"),
    ] {
        fs::create_dir_all(path).expect("an M5 directory is created");
    }
    symlink(&inside_share, root.join("alias")).expect("the alias is made");
    let capitals = software.join("SHARE");
    let insensitive = fs::metadata(&capitals).is_ok();
    let mut cases = vec![
        ("the share itself", share.clone(), true),
        ("a directory inside the share", inside_share.clone(), true),
        (
            "the software directory, which holds the share and the holding directory",
            software.clone(),
            true,
        ),
        ("the holding directory itself", holding.clone(), true),
        (
            "a directory inside the holding directory",
            inside_holding,
            true,
        ),
        (
            "a symbolic link to a directory inside the share",
            root.join("alias"),
            true,
        ),
        (
            "a sibling of the share in the software directory",
            software.join("sibling"),
            false,
        ),
        (
            "a directory outside the software directory",
            root.join("elsewhere"),
            false,
        ),
    ];
    if insensitive {
        cases.push((
            "the share's path in capitals, on this case-insensitive volume",
            capitals,
            true,
        ));
    }
    let answers: Vec<io::Result<bool>> = cases
        .iter()
        .map(|(_, source, _)| check(&share, &holding, source))
        .collect();
    let lines = cases
        .iter()
        .zip(&answers)
        .map(|((case, _, must_refuse), answer)| {
            let answer = match answer {
                Ok(true) => "refused".to_string(),
                Ok(false) => "accepted".to_string(),
                Err(error) => format!("error: {error}"),
            };
            let wanted = if *must_refuse { "refuse" } else { "accept" };
            format!("{case}: {answer}; a correct check must {wanted} it")
        })
        .collect();
    let cases: Vec<SourceCase> = cases
        .iter()
        .map(|(case, _, must_refuse)| SourceCase {
            name: case.to_string(),
            must_refuse: *must_refuse,
        })
        .collect();
    let verdict = gate::judge_m5(&cases, &answers);
    write_record(&Record {
        run: name.to_string(),
        configuration: json!({
            "check": what,
            "volume": "this Mac's temporary directory",
            "case_insensitive": insensitive,
        }),
        digests: BTreeMap::from([("test commit".to_string(), test_commit())]),
        verdicts: BTreeMap::from([("M5".to_string(), verdict.clone())]),
        observations: BTreeMap::new(),
        counts: BTreeMap::new(),
        notes: Vec::new(),
        lines,
    });
    verdict
}

#[test]
#[ignore = "a gate measurement: run it with the others so the results file has its record"]
fn m5_catches_a_check_that_accepts_a_source_inside_the_share() {
    let verdict = run_m5(
        "m5-source-inside-share",
        "refuse only when the share or the holding directory lies inside the source",
        accepts_a_source_inside_the_share,
    );

    assert_caught("M5", &verdict);
}

#[test]
#[ignore = "a gate measurement: run it with the others so the results file has its record"]
fn m5_catches_a_check_that_accepts_a_share_inside_a_source() {
    let verdict = run_m5(
        "m5-share-inside-source",
        "refuse only when the source lies inside the share or the holding directory",
        accepts_a_share_inside_a_source,
    );

    assert_caught("M5", &verdict);
}

#[test]
#[ignore = "a gate measurement: run it with the others so the results file has its record"]
fn m5_refuses_a_source_that_overlaps_the_share_or_the_holding_directory_either_way() {
    let verdict = run_m5(
        "m5-identity",
        "gate::refuses_source: overlaps by device and inode, both ways, against the share and the holding directory",
        gate::refuses_source,
    );

    assert_eq!(verdict, Verdict::Pass);
}

fn run_spec(run: &str, what: &str, expect: Expect) -> RunSpec {
    RunSpec {
        run: run.to_string(),
        what: what.to_string(),
        expect,
    }
}

fn row(id: &str, claim: &str, runs: Vec<RunSpec>) -> RowSpec {
    RowSpec {
        id: id.to_string(),
        claim: claim.to_string(),
        runs,
    }
}

const PLAIN: &str =
    "a read-only LINUX_COMPLETE share with no override attribute, as Q1(a) plans it";
const OWNED: &str = "the same share with every entry carrying override_stat 0:0:<mode>";

fn rows() -> Vec<RowSpec> {
    use Expect::{Fail, Observe, Pass};
    vec![
        row(
            "M1",
            "As planned: uid 1000 sees the share's root and a file in it owned by 0:0.",
            vec![
                run_spec("share-plain", PLAIN, Pass),
                run_spec(
                    "share-plain-simplified",
                    "the share served LINUX_SIMPLIFIED",
                    Fail,
                ),
            ],
        ),
        row(
            "M1 (override_stat)",
            "M1 with every host entry carrying user.containers.override_stat 0:0:<mode>.",
            vec![
                run_spec("share-owned", OWNED, Pass),
                run_spec(
                    "share-owned-simplified",
                    "the same, served LINUX_SIMPLIFIED",
                    Fail,
                ),
            ],
        ),
        row(
            "M2",
            "As planned: with no ro mount, a uid 1000 create fails EACCES, a uid 0 create fails EROFS, and the options hold no ro.",
            vec![
                run_spec("share-plain", PLAIN, Pass),
                run_spec("share-plain-ro", "the share mounted ro", Fail),
                run_spec(
                    "share-plain-writable",
                    "the share added with read_only false",
                    Fail,
                ),
            ],
        ),
        row(
            "M2 (override_stat)",
            "M2 on the share whose entries carry override_stat 0:0:<mode>.",
            vec![
                run_spec("share-owned", OWNED, Pass),
                run_spec("share-owned-ro", "the same, mounted ro", Fail),
                run_spec(
                    "share-owned-writable",
                    "the same, added with read_only false",
                    Fail,
                ),
            ],
        ),
        row(
            "M3",
            "A descriptor, a mapping and a running binary opened before the host moves the tree to the holding directory each read their bytes after it, with lane-vmm confined and its self-check passed first. The profile is (allow default): it denies every file write but the console's and the disk image's, and denies reads only inside the lane directory, apart from the software directory, the console, the image and the initramfs.",
            vec![
                run_spec("detach-move", Host::Move.describe(), Pass),
                run_spec("detach-delete", Host::Delete.describe(), Fail),
            ],
        ),
        row(
            "M3 holders",
            "Which of M3's three holders read their bytes back under each host, the delete mutant included.",
            vec![
                run_spec("detach-move", Host::Move.describe(), Observe),
                run_spec("detach-delete", Host::Delete.describe(), Observe),
            ],
        ),
        row(
            "M4",
            "5 s after the host's last move, a working directory held in the tree lists empty, and every former name and the tree's path fail ENOENT.",
            vec![
                run_spec("detach-move", Host::Move.describe(), Pass),
                run_spec("detach-leave-dirs", Host::LeaveDirs.describe(), Fail),
                run_spec("detach-remove-dirs", Host::RemoveDirs.describe(), Fail),
            ],
        ),
        row(
            "M5",
            "An identity check refuses a Mount source that overlaps the share or the holding directory, in either direction.",
            vec![
                run_spec("m5-identity", "gate::refuses_source", Pass),
                run_spec(
                    "m5-source-inside-share",
                    "a check that accepts a source inside the share",
                    Fail,
                ),
                run_spec(
                    "m5-share-inside-source",
                    "a check that accepts a share inside a source",
                    Fail,
                ),
            ],
        ),
        row(
            "R1",
            "027 §2, item 2: a running process finds a subtree the host creates in a share it already has mounted, with no new mount (80 of 80).",
            vec![run_spec("r1-created-subtree", "80 trials", Pass)],
        ),
        row(
            "R2 visible",
            "027 §3.4, item 4: the control. A bind made in the initial namespace on a shared mount reaches the workload's slave copy, and the workload's stat finds the file.",
            vec![run_spec(
                "r2-staging",
                "guest root stages each configuration",
                Pass,
            )],
        ),
        row(
            "R2 private",
            "027 §3.4, item 4: with the workload's copy private, mount returns 0 and the workload's stat fails ENOENT.",
            vec![run_spec(
                "r2-staging",
                "guest root stages each configuration",
                Pass,
            )],
        ),
        row(
            "R2 unreachable",
            "027 §3.4, item 4: a bind from a source the workload's namespace cannot reach returns 0, and the workload's stat fails ENOENT.",
            vec![run_spec(
                "r2-staging",
                "guest root stages each configuration",
                Pass,
            )],
        ),
        row(
            "R2 private-parent",
            "027 §3.4, item 4: one of the two configurations 027 counts and does not name; that these two are 027's rests on the original measurement, outside this repository. With the parent mount private, the attach point made before the workload's namespace, and that namespace left as unshare made it, mount returns 0 and the workload's stat fails ENOENT. Its line differs from late-directory's only in the names, `attach_point` and the staged file's inode.",
            vec![run_spec(
                "r2-staging",
                "guest root stages each configuration",
                Pass,
            )],
        ),
        row(
            "R2 late-directory",
            "027 §3.4, item 4: the other of 027's two unnamed configurations, by the same measurement outside this repository. With the parent mount private and no attach point made beforehand, the initial namespace creates the directory after the workload's namespace exists and binds onto it; mount returns 0 and the workload's stat fails ENOENT. Both parents are private, and the workload stats only `opt/p/bin/x`, never `opt/p`, so the record cannot tell whether the late attach point changes anything.",
            vec![run_spec(
                "r2-staging",
                "guest root stages each configuration",
                Pass,
            )],
        ),
        row(
            "R3",
            "027 §4, item 9: a held descriptor and mapping read their bytes after umount2(MNT_DETACH) (16 of 16).",
            vec![run_spec("r3-lazy-unmount", "16 trials", Pass)],
        ),
        row(
            "R4 thread",
            "027 §4: after unshare(CLONE_FS | CLONE_FILES), a thread's descriptor and working directory show only under /proc/<pid>/task/<tid>/ (6 of 6).",
            vec![run_spec("r4-tables", "6 trials of each part", Pass)],
        ),
        row(
            "R4 in flight",
            "027 §4: a descriptor in flight over SCM_RIGHTS is in no table (6 of 6).",
            vec![run_spec("r4-tables", "6 trials of each part", Pass)],
        ),
        row(
            "Names",
            "What the guest sees for 8 name pairs APFS treats as one name on the host.",
            vec![run_spec("share-plain", PLAIN, Observe)],
        ),
        row(
            "Cache window",
            "The detach probes with no wait after the host's flag.",
            vec![run_spec("detach-nowait", Host::Move.describe(), Observe)],
        ),
        row(
            "Setuid",
            "What override_stat's setuid bit does for a binary the workload runs. Only the setuid bit was measured; neither setgid nor nodev was.",
            vec![
                run_spec("share-owned", OWNED, Observe),
                run_spec("share-owned-nosuid", "the same, mounted nosuid", Observe),
            ],
        ),
    ]
}

fn command_text(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|error| format!("{program} failed: {error}"))
}

fn records_written() -> String {
    let mut times: Vec<u64> = fs::read_dir(records_dir())
        .expect("the records directory lists")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| fs::metadata(path).and_then(|found| found.modified()).ok())
        .filter_map(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .collect();
    times.sort_unstable();
    let utc = |secs: &u64| {
        command_text(
            "date",
            &["-u", "-r", &secs.to_string(), "+%Y-%m-%d %H:%M:%S UTC"],
        )
    };
    match (times.first(), times.last()) {
        (Some(first), Some(last)) => format!("{} to {}", utc(first), utc(last)),
        _ => "no record".to_string(),
    }
}

fn preamble(records: &[Record]) -> String {
    let commit = one_commit(records)
        .unwrap_or_else(|why| panic!("the records are not one run of one commit: {why}"));
    let written = records_written();
    let host = format!(
        "macOS {} ({}), {}",
        command_text("sw_vers", &["-productVersion"]),
        command_text("sw_vers", &["-buildVersion"]),
        command_text("uname", &["-m"])
    );
    format!(
        "# Spec 027's Q1(a) gate on the pinned guest

This file holds the measurements behind spec 027's Q1(a) gate (acceptance item 17), and re-runs of
four claims 027 measured outside this repository (items 2, 4 and 9). Each ran on this repository's
test lanes: the pinned libkrun v1.19.4 bundle `eed141b574337d99` and its Linux 6.12.91 kernel,
with no cell. The ignored tests in `crates/plasmosome-lanes/tests/q1a_gate.rs` and
`tests/rerun.rs` ran them, and `write_the_results_file` wrote this file from their records.

A row passes when its configuration passed and every mutant of it failed. A row whose run left
no record, or decided nothing, reads `not run`, a mutant's run included. An observation is
recorded and not judged, and an observation row whose run recorded nothing for it reads
`not recorded`. A tallied row shows its measured count beside its verdict.

Every workload probe ran as uid 1000, and every result line carries its process's effective uid
as `id_u`, except that a `release` line the holder answered carries the holder's uid, passed on,
not the release process's own. The setuid observation's holder is the one workload process that
ran with effective uid 0, which is what that observation looks at. On macOS a new directory takes
the group of the directory it is made in, so the group M1's plain run shows is that of the folder
the lane was made in, not one the share chose.

Owner gate O-6 was open while these ran: a library planted in `/opt/homebrew/opt/llvm/lib` or
`/usr/local/lib` would load before `lane-vmm` checks any image. Nothing was planted there, and
neither directory was changed. Nothing here needed root on the host or a second host uid (owner
gate O-8).

- Host: {host}.
- Test commit: `{commit}`.
- Records written: {written}. The writer refuses records from more than one commit, or from a
  tree with uncommitted changes.
- Re-run: build the guest binaries as `crates/plasmosome-lanes/README.md` says, then
  `cargo test -p plasmosome-lanes --test q1a_gate --test rerun -- --ignored --test-threads=1 --skip write_the_results_file`.
  The writer runs last: `cargo test -p plasmosome-lanes --test q1a_gate write_the_results_file -- --ignored`.
",
    )
}

fn scratch_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=lanes",
            "-c",
            "user.email=lanes@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

#[test]
fn the_test_commit_is_marked_uncommitted_for_an_untracked_or_a_changed_file() {
    let repo = tempfile::tempdir().expect("a directory for a repository");
    let crate_dir = repo.path().join("crate");
    write_file(
        &repo.path().join(".git"),
        b"gitdir: no-such-directory\n",
        0o644,
    );
    assert!(
        commit_at(repo.path()).ends_with(" with uncommitted changes"),
        "a directory git cannot read is not clean"
    );
    fs::remove_file(repo.path().join(".git")).expect("the .git file is removed");
    scratch_git(repo.path(), &["init", "-q"]);
    write_file(&repo.path().join("tracked"), b"tracked\n", 0o644);
    write_file(&crate_dir.join("lib"), b"lib\n", 0o644);
    scratch_git(repo.path(), &["add", "tracked", "crate/lib"]);
    scratch_git(repo.path(), &["commit", "-q", "-m", "one"]);

    let head = commit_at(&crate_dir);
    assert!(
        head.len() == 40 && head.chars().all(|c| c.is_ascii_hexdigit()),
        "{head}"
    );

    let untracked = repo.path().join(".cargo");
    fs::create_dir(&untracked).expect("an untracked directory is made");
    write_file(
        &untracked.join("config.toml"),
        b"[build]\nrustflags = [\"--cfg\", \"untracked\"]\n",
        0o644,
    );
    assert_eq!(
        commit_at(&crate_dir),
        format!("{head} with uncommitted changes"),
        "an untracked file at the repository's root, read from a subdirectory"
    );

    fs::remove_dir_all(&untracked).expect("the untracked directory is removed");
    assert_eq!(commit_at(&crate_dir), head);
    write_file(&repo.path().join("tracked"), b"changed\n", 0o644);
    assert_eq!(
        commit_at(&crate_dir),
        format!("{head} with uncommitted changes"),
        "a changed file at the repository's root, read from a subdirectory"
    );
}

#[test]
fn the_test_commit_sees_an_untracked_file_in_a_repository_set_to_hide_them() {
    let repo = tempfile::tempdir().expect("a directory for a repository");
    scratch_git(repo.path(), &["init", "-q"]);
    scratch_git(repo.path(), &["config", "status.showUntrackedFiles", "no"]);
    write_file(&repo.path().join("tracked"), b"tracked\n", 0o644);
    scratch_git(repo.path(), &["add", "tracked"]);
    scratch_git(repo.path(), &["commit", "-q", "-m", "one"]);
    let head = commit_at(repo.path());

    write_file(
        &repo.path().join(".cargo").join("config.toml"),
        b"[build]\nrustflags = [\"--cfg\", \"untracked\"]\n",
        0o644,
    );

    assert_eq!(
        commit_at(repo.path()),
        format!("{head} with uncommitted changes")
    );
}

#[test]
#[ignore = "reads the records the gate lanes and the re-runs wrote"]
fn write_the_results_file() {
    let records = read_records();

    let text = render(&preamble(&records), &rows(), &records);

    let path = std::env::var_os("LANES_RESULTS")
        .map_or_else(|| records_dir().with_file_name(RESULTS_FILE), PathBuf::from);
    fs::write(&path, text).expect("the results file is written");
    eprintln!("results written to {}", path.display());
}
