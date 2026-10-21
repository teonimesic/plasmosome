use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use plasmosome_lanes::bundle::sha256_hex;
use plasmosome_lanes::config::{Confinement, GuestMount, MountOption, Probe, Semantics, Share};
use plasmosome_lanes::console::{ProbeResult, result_line};
use plasmosome_lanes::gate::{
    CTL_MOUNT, CTL_TAG, Expected, HOLDING, SHARE_MOUNT, SHARE_TAG, TREE_DATA, TREE_DOC, TREE_TOOL,
    Verdict,
};
use plasmosome_lanes::results::Record;
use plasmosome_lanes::run::{LaneRun, Outcome};
use plasmosome_lanes::steps::{Step, StepsRun, run_lane_with_steps};
use serde_json::{Value, json};

use super::guest::{
    Lane, bundle_dir, guest_binary, guest_binary_path, lane, manifest_dir, report, signed_vmm,
    write_initramfs,
};

pub const DATA: &[u8] = b"held by the workload\n";
pub const DOC: &[u8] = b"a file two directories deep\n";
pub const OVERRIDE_STAT: &str = "user.containers.override_stat";

/// A gate lane: a lane whose software directory holds the exported share, the holding directory
/// beside it, and the read-only flag share.
pub struct GateLane {
    pub lane: Lane,
    pub software: PathBuf,
    pub share: PathBuf,
    pub holding: PathBuf,
    pub ctl: PathBuf,
    pub profile: Option<String>,
}

pub fn gate_lane(
    run_id: &str,
    probes: Vec<Probe>,
    semantics: Semantics,
    read_only: bool,
    options: Vec<MountOption>,
) -> GateLane {
    let mut lane = lane(run_id, probes);
    let software = lane.root().join("software");
    let share = software.join("share");
    let holding = software.join(HOLDING);
    let ctl = software.join("ctl");
    for dir in [&share, &holding, &ctl] {
        fs::create_dir_all(dir).expect("a software directory is created");
    }
    lane.lane.shares.push(Share {
        tag: SHARE_TAG.to_string(),
        path: share.clone(),
        shm_size: 0,
        read_only,
        semantics,
    });
    lane.lane.shares.push(Share {
        tag: CTL_TAG.to_string(),
        path: ctl.clone(),
        shm_size: 0,
        read_only: true,
        semantics: Semantics::Complete,
    });
    lane.guest.mounts.push(GuestMount {
        tag: SHARE_TAG.to_string(),
        path: PathBuf::from(SHARE_MOUNT),
        options,
    });
    lane.guest.mounts.push(GuestMount {
        tag: CTL_TAG.to_string(),
        path: PathBuf::from(CTL_MOUNT),
        options: Vec::new(),
    });
    GateLane {
        lane,
        software,
        share,
        holding,
        ctl,
        profile: None,
    }
}

pub fn write_file(path: &Path, bytes: &[u8], mode: u32) {
    fs::create_dir_all(path.parent().expect("a placed file has a parent"))
        .expect("the placed file's directory is created");
    fs::write(path, bytes).expect("the placed file is written");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("the mode is set");
}

pub fn expected(bytes: &[u8]) -> Expected {
    Expected {
        sha256: sha256_hex(bytes),
        bytes: bytes.len() as u64,
    }
}

/// Place the tree a detach removes, and return what its data file and its command must read back.
pub fn place_tree(share: &Path) -> (Expected, Expected) {
    let tool = guest_binary("lane-probe");
    write_file(&share.join(TREE_TOOL), &tool, 0o755);
    write_file(&share.join(TREE_DATA), DATA, 0o644);
    write_file(&share.join(TREE_DOC), DOC, 0o644);
    (expected(DATA), expected(&tool))
}

fn entries(path: &Path, found: &mut Vec<PathBuf>) {
    found.push(path.to_path_buf());
    if fs::symlink_metadata(path).expect("a placed entry").is_dir() {
        let mut children: Vec<PathBuf> = fs::read_dir(path)
            .expect("a placed directory lists")
            .map(|entry| entry.expect("a placed entry").path())
            .collect();
        children.sort();
        for child in children {
            entries(&child, found);
        }
    }
}

/// Every entry under `root`, `root` first.
pub fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    entries(root, &mut found);
    found
}

pub fn set_override(paths: &[PathBuf], value: &str) {
    if paths.is_empty() {
        return;
    }
    let output = Command::new("/usr/bin/xattr")
        .args(["-w", OVERRIDE_STAT, value])
        .args(paths)
        .output()
        .expect("xattr runs");
    assert!(
        output.status.success(),
        "xattr -w {OVERRIDE_STAT} {value} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Mark every entry under `root` owned by 0:0 with its own mode, through libkrun's
/// `user.containers.override_stat` attribute.
pub fn own_as_root(root: &Path) {
    let mut by_mode: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
    for path in walk(root) {
        let mode = fs::symlink_metadata(&path).expect("a placed entry").mode() & 0o7777;
        by_mode.entry(mode).or_default().push(path);
    }
    for (mode, paths) in by_mode {
        set_override(&paths, &format!("0:0:{mode:04o}"));
    }
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// Confine `lane-vmm` with an SBPL profile that lets it read the software directory and nothing
/// else in the lane's directory but its console, initramfs and root image, and write only the
/// console and the root image. The self-check reads and writes the software directory as
/// `allowed`, and a sibling `outside` directory.
pub fn confine(gate: &mut GateLane) {
    let root = gate.lane.root();
    let outside = root.join("outside");
    write_file(&outside.join("secret"), b"secret\n", 0o644);
    let config = &gate.lane.lane;
    let profile = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write* (literal {console}) (literal {image}))\n(deny file-read* (subpath {root}))\n(allow file-read* (subpath {software}) (literal {console}) (literal {image}) (literal {initramfs}))\n",
        console = quoted(&config.console),
        image = quoted(&config.root_image),
        root = quoted(&root),
        software = quoted(&gate.software),
        initramfs = quoted(&config.initramfs),
    );
    let path = root.join("lane.sb");
    fs::write(&path, &profile).expect("the profile is saved");
    gate.lane.lane.confinement = Some(Confinement {
        profile: path,
        allowed: gate.software.clone(),
        outside,
    });
    gate.profile = Some(profile.replace(&root.display().to_string(), "<lane>"));
}

pub fn run_gate(gate: &GateLane, steps: Vec<Step>, timeout: Duration) -> StepsRun {
    write_initramfs(&gate.lane);
    let run = run_lane_with_steps(
        signed_vmm(),
        &gate.lane.lane,
        &gate.lane.guest,
        timeout,
        steps,
    )
    .expect("lane-vmm starts");
    report(&run.run);
    for step in &run.steps {
        eprintln!("step after `{}`: {step:?}", step.after);
    }
    run
}

pub fn results(run: &LaneRun) -> &[ProbeResult] {
    match &run.outcome {
        Outcome::Finished(results) => results,
        _ => &[],
    }
}

fn outcome_text(run: &LaneRun) -> String {
    let outcome = match &run.outcome {
        Outcome::Finished(results) => format!("finished with {} results", results.len()),
        Outcome::TimedOut => "timed out".to_string(),
        Outcome::NotRun(why) => format!("not run: {why}"),
    };
    format!("outcome: {outcome}; lane-vmm exit status {:?}", run.status)
}

fn relative(lane: &Lane, path: &Path) -> String {
    path.strip_prefix(lane.root()).map_or_else(
        |_| path.display().to_string(),
        |path| format!("<lane>/{}", path.display()),
    )
}

/// What a gate lane ran with, for the results file, with the lane's directory written `<lane>`.
pub fn configuration(gate: &GateLane, layout: &[&str]) -> Value {
    let lane = &gate.lane;
    let shares: Vec<Value> = lane
        .lane
        .shares
        .iter()
        .map(|share| {
            json!({"tag": share.tag, "path": relative(lane, &share.path),
                   "read_only": share.read_only, "semantics": share.semantics})
        })
        .collect();
    let mounts: Vec<Value> = lane
        .guest
        .mounts
        .iter()
        .map(|mount| json!({"tag": mount.tag, "path": mount.path, "options": mount.options}))
        .collect();
    json!({
        "vcpus": lane.lane.vcpus,
        "memory_mib": lane.lane.memory_mib,
        "shares": shares,
        "guest_mounts": mounts,
        "confinement": gate.profile.as_ref().map(|profile| profile.lines().collect::<Vec<_>>()),
        "layout": layout,
        "probes": lane.guest.probes.len(),
    })
}

fn bundle_sums() -> BTreeMap<String, String> {
    let text = fs::read_to_string(bundle_dir().join("SHA256SUMS")).expect("SHA256SUMS is read");
    text.lines()
        .filter_map(|line| line.split_once("  "))
        .map(|(digest, path)| (format!("bundle {path}"), digest.to_string()))
        .collect()
}

pub fn test_commit() -> String {
    commit_at(&manifest_dir())
}

pub fn commit_at(dir: &Path) -> String {
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git runs");
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let clean = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .current_dir(dir)
        .status()
        .expect("git runs")
        .success();
    if clean {
        head
    } else {
        format!("{head} with uncommitted changes")
    }
}

fn file_digest(path: &Path) -> String {
    sha256_hex(&fs::read(path).expect("a digested file is read"))
}

/// The digests a lane's record carries: every bundle file as `SHA256SUMS` lists it, checked
/// before the run, the lane's initramfs, the guest binaries, `lane-vmm` and the test commit.
pub fn lane_digests(lane: &Lane) -> BTreeMap<String, String> {
    let mut digests = bundle_sums();
    digests.insert("initramfs".into(), file_digest(&lane.lane.initramfs));
    for name in ["lane-init", "lane-probe"] {
        digests.insert(name.into(), file_digest(&guest_binary_path(name)));
    }
    digests.insert("lane-vmm".into(), file_digest(signed_vmm()));
    digests.insert("test commit".into(), test_commit());
    digests
}

/// The framed lines of a run's console, as `lane-init` wrote them.
pub fn framed_lines(run: &LaneRun) -> Vec<String> {
    match &run.outcome {
        Outcome::Finished(results) => results.iter().map(result_line).collect(),
        _ => run
            .console
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| line.starts_with("LANE-"))
            .map(str::to_string)
            .collect(),
    }
}

/// The notes a lane's record carries: how the run ended, `lane-vmm`'s own stderr lines and each
/// host step.
pub fn lane_notes(run: &StepsRun) -> Vec<String> {
    let mut notes = vec![outcome_text(&run.run)];
    notes.extend(
        run.run
            .stderr
            .lines()
            .filter(|line| line.starts_with("lane-vmm:"))
            .map(str::to_string),
    );
    for step in &run.steps {
        notes.push(match &step.outcome {
            Ok(text) => format!(
                "host step after `{}` at {} ms, flag written at {} ms: ok: {text}",
                step.after, step.started_ms, step.flagged_ms
            ),
            Err(error) => format!(
                "host step after `{}` at {} ms, ended at {} ms with no flag written: error: {error}",
                step.after, step.started_ms, step.flagged_ms
            ),
        });
    }
    notes
}

pub fn records_dir() -> PathBuf {
    std::env::var_os("LANES_RECORDS").map_or_else(
        || Path::new(env!("CARGO_TARGET_TMPDIR")).join("q1a-records"),
        PathBuf::from,
    )
}

pub fn write_record(record: &Record) {
    let dir = records_dir();
    fs::create_dir_all(&dir).expect("the records directory is created");
    let path = dir.join(format!("{}.json", record.run));
    fs::write(
        &path,
        serde_json::to_vec_pretty(record).expect("the record serializes"),
    )
    .expect("the record is written");
    eprintln!("record written to {}", path.display());
}

pub fn read_records() -> Vec<Record> {
    let dir = records_dir();
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("no records in {}: {error}", dir.display()))
        .map(|entry| entry.expect("a record entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| {
            serde_json::from_slice(&fs::read(path).expect("a record is read"))
                .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()))
        })
        .collect()
}

/// What a gate lane run showed: a verdict per row, observations per row, the measured count of a
/// tallied row, and notes beyond the run's own.
#[derive(Default)]
pub struct Shown {
    pub verdicts: BTreeMap<String, Verdict>,
    pub observations: BTreeMap<String, Vec<String>>,
    pub counts: BTreeMap<String, String>,
    pub notes: Vec<String>,
}

/// Record a gate lane run with what it showed, with the lane's directory written `<lane>`.
pub fn record_lane(
    name: &str,
    gate: &GateLane,
    layout: &[&str],
    run: &StepsRun,
    shown: Shown,
) -> Record {
    let root = gate.lane.root().display().to_string();
    let notes = lane_notes(run)
        .into_iter()
        .chain(shown.notes)
        .map(|note| note.replace(&root, "<lane>"))
        .collect();
    let record = Record {
        run: name.to_string(),
        configuration: configuration(gate, layout),
        digests: lane_digests(&gate.lane),
        verdicts: shown.verdicts,
        observations: shown.observations,
        counts: shown.counts,
        notes,
        lines: framed_lines(&run.run),
    };
    write_record(&record);
    record
}

/// A measurement decided: it passed or failed, and never `not run`.
pub fn assert_decided(row: &str, verdict: &Verdict) {
    eprintln!("{row}: {verdict:?}");
    assert!(
        !matches!(verdict, Verdict::NotRun(_)),
        "{row} did not run: {verdict:?}"
    );
}

/// A mutant was caught: the measurement failed on it.
pub fn assert_caught(row: &str, verdict: &Verdict) {
    eprintln!("{row} on its mutant: {verdict:?}");
    assert!(
        matches!(verdict, Verdict::Fail(_)),
        "{row} did not catch its mutant: {verdict:?}"
    );
}

/// Each non-ASCII character as `U+XXXX`, so a name's code points read plainly.
pub fn spelled(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_string()
            } else {
                format!("<U+{:04X}>", c as u32)
            }
        })
        .collect()
}
