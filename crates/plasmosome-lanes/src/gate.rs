use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Probe, WORKLOAD_UID};
use crate::console::ProbeResult;

/// The tag of the software share every gate lane exports.
pub const SHARE_TAG: &str = "sw";
/// Where the guest mounts the software share.
pub const SHARE_MOUNT: &str = "/mnt/sw";
/// The tag of the read-only share that carries the host's step flags.
pub const CTL_TAG: &str = "ctl";
/// Where the guest mounts the flag share.
pub const CTL_MOUNT: &str = "/mnt/ctl";
/// The holding directory's name, beside the exported share in the software directory.
pub const HOLDING: &str = "holding";
/// The tree a detach removes, relative to the share.
pub const TREE: &str = "opt/p";
/// A data file in the tree, relative to the share.
pub const TREE_DATA: &str = "opt/p/lib/data.txt";
/// A command in the tree, a copy of `lane-probe`, relative to the share.
pub const TREE_TOOL: &str = "opt/p/bin/tool";
/// A file two directories deep in the tree, relative to the share.
pub const TREE_DOC: &str = "opt/p/share/doc/readme.txt";
/// libkrun v1.19.4's entry and attribute cache timeout under `LINUX_COMPLETE`.
pub const CACHE_WINDOW_MS: u64 = 5000;
/// How long an `await` probe waits for a host flag.
pub const AWAIT_MS: u64 = 30_000;
/// The probe after whose result the host detaches the tree.
pub const DETACH_AFTER: &str = "m4-hold-cwd";
/// The flag the host writes once the tree is detached.
pub const DETACH_FLAG: &str = "detached";
/// The uid of guest root.
pub const ROOT_UID: u32 = 0;

/// How a measurement or one of its mutants came out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict", content = "reason")]
pub enum Verdict {
    /// The run showed what the measurement claims.
    Pass,
    /// The run contradicted it, with what was seen.
    Fail(String),
    /// The run gave no evidence either way, with why. Never a pass.
    NotRun(String),
}

/// The bytes a holder must read back: their SHA-256 and their length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    pub sha256: String,
    pub bytes: u64,
}

/// One trial of a re-run: its index, whether it showed the claim, and its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trial {
    pub index: usize,
    pub outcome: Verdict,
}

/// The trials of a re-run, judged against the number the claim was measured with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally {
    pub claimed: usize,
    pub trials: Vec<Trial>,
}

impl Tally {
    /// How many trials showed the claim.
    pub fn held(&self) -> usize {
        self.trials
            .iter()
            .filter(|trial| trial.outcome == Verdict::Pass)
            .count()
    }

    /// `Pass` only when every claimed trial ran and showed the claim; `Fail` when any trial
    /// contradicted it; otherwise `NotRun`. The count is never rounded up.
    pub fn verdict(&self) -> Verdict {
        Verdict::NotRun(format!("{} of {}", self.held(), self.claimed))
    }
}

fn probe(id: impl Into<String>, uid: u32, args: &[&str]) -> Probe {
    Probe {
        id: id.into(),
        uid,
        args: args.iter().map(|arg| arg.to_string()).collect(),
    }
}

fn in_share(relative: &str) -> String {
    format!("{SHARE_MOUNT}/{relative}")
}

fn flag(name: &str) -> String {
    format!("{CTL_MOUNT}/{name}")
}

/// M1's probes: the workload stats the share's root and a file in it.
pub fn m1_probes() -> Vec<Probe> {
    vec![
        probe("m1-share", WORKLOAD_UID, &["stat", SHARE_MOUNT]),
        probe("m1-file", WORKLOAD_UID, &["stat", &in_share(TREE_TOOL)]),
    ]
}

/// M1 passes when the workload sees the share's root and the file owned by 0:0.
pub fn judge_m1(results: &[ProbeResult]) -> Verdict {
    let _ = results;
    Verdict::NotRun("not judged".to_string())
}

/// M2's probes: a workload create, a guest-root create and the mount table.
pub fn m2_probes() -> Vec<Probe> {
    vec![
        probe(
            "m2-workload-create",
            WORKLOAD_UID,
            &["create", &in_share("opt/m2-workload")],
        ),
        probe(
            "m2-root-create",
            ROOT_UID,
            &["create", &in_share("opt/m2-root")],
        ),
        probe("m2-mountinfo", WORKLOAD_UID, &["mountinfo"]),
    ]
}

/// M2 passes when the workload's create fails with exactly `EACCES`, guest root's with exactly
/// `EROFS`, and the share's mount options hold no `ro`.
pub fn judge_m2(results: &[ProbeResult]) -> Verdict {
    let _ = results;
    Verdict::NotRun("not judged".to_string())
}

/// The detach lane's probes for M3 and M4: a descriptor, a mapping, a running executable and a
/// working directory held in the tree; the wait for the host's detach; with `wait`, a sleep of
/// the cache window; then each holder released, and the share listed.
pub fn detach_probes(wait: bool) -> Vec<Probe> {
    let data = in_share(TREE_DATA);
    let mut probes = vec![
        probe("m3-hold-fd", WORKLOAD_UID, &["hold-fd", "fd", &data]),
        probe("m3-hold-map", WORKLOAD_UID, &["hold-map", "map", &data]),
        probe(
            "m3-hold-exec",
            WORKLOAD_UID,
            &["hold-exec", "exec", &in_share(TREE_TOOL)],
        ),
        probe(DETACH_AFTER, WORKLOAD_UID, &["hold-cwd", "cwd", &in_share(TREE)]),
        probe(
            "detach-await",
            WORKLOAD_UID,
            &["await", &flag(DETACH_FLAG), &AWAIT_MS.to_string()],
        ),
    ];
    if wait {
        probes.push(probe(
            "detach-wait",
            WORKLOAD_UID,
            &["sleep", &CACHE_WINDOW_MS.to_string()],
        ));
    }
    probes.extend([
        probe("m4-release-cwd", WORKLOAD_UID, &["release", "cwd"]),
        probe("m3-release-fd", WORKLOAD_UID, &["release", "fd"]),
        probe("m3-release-map", WORKLOAD_UID, &["release", "map"]),
        probe("m3-release-exec", WORKLOAD_UID, &["release", "exec"]),
        probe("detach-list-share", WORKLOAD_UID, &["list", SHARE_MOUNT]),
        probe("detach-list-opt", WORKLOAD_UID, &["list", &in_share("opt")]),
        probe("detach-lookup-tree", WORKLOAD_UID, &["lookup", &in_share(TREE)]),
    ]);
    probes
}

/// The four confinement checks `lane-vmm` reports on stderr once confined, in order.
pub const SELF_CHECKS: [&str; 4] = [
    "read inside",
    "write inside",
    "read outside",
    "write outside",
];

/// M3 passes when the helper's unconfined control and confined self-check each held, the host's
/// flag arrived, and the descriptor, the mapping and the executable each read back `data` and
/// `tool` after the detach, while the share lists no holding directory.
pub fn judge_m3(
    results: &[ProbeResult],
    stderr: &str,
    data: &Expected,
    tool: &Expected,
) -> Verdict {
    let _ = (results, stderr, data, tool);
    Verdict::NotRun("not judged".to_string())
}

/// M4 passes when, once the cache window has passed if `waited`, the working directory held
/// inside the detached tree lists empty, every name it held fails with `ENOENT`, and a lookup of
/// the tree by path fails with `ENOENT`.
pub fn judge_m4(results: &[ProbeResult], waited: bool) -> Verdict {
    let _ = (results, waited);
    Verdict::NotRun("not judged".to_string())
}

/// One candidate Mount source for M5, and whether a correct check must refuse it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCase {
    pub name: String,
    pub must_refuse: bool,
}

/// M5 passes when `refused` names every case that must be refused, and no other. `refused` holds
/// the check's answer for each case in order, or the error it gave.
pub fn judge_m5(cases: &[SourceCase], refused: &[io::Result<bool>]) -> Verdict {
    let _ = (cases, refused);
    Verdict::NotRun("not judged".to_string())
}

/// The reference check M5 measures: refuse `source` when it overlaps the share or the holding
/// directory, by identity, in either direction.
pub fn refuses_source(share: &Path, holding: &Path, source: &Path) -> io::Result<bool> {
    let _ = (share, holding, source);
    Ok(false)
}

/// R1's probe for trial `index`: the process the host's creation must reach.
pub fn r1_watch(index: usize) -> String {
    format!("r1-watch-{index}")
}

/// R1's flag for trial `index`.
pub fn r1_flag(index: usize) -> String {
    format!("r1-{index}")
}

/// The file the host creates in trial `index` of R1, relative to the share.
pub fn r1_path(index: usize) -> String {
    format!("opt/r1-{index}/bin/x")
}

/// R1's probes: per trial, a process that looks the path up before the host creates it, the wait
/// for the host's flag, and the same process's lookup after.
pub fn r1_probes(trials: usize) -> Vec<Probe> {
    (0..trials)
        .flat_map(|index| {
            let name = format!("w{index}");
            [
                probe(
                    r1_watch(index),
                    WORKLOAD_UID,
                    &["hold-watch", &name, &in_share(&r1_path(index))],
                ),
                probe(
                    format!("r1-await-{index}"),
                    WORKLOAD_UID,
                    &["await", &flag(&r1_flag(index)), &AWAIT_MS.to_string()],
                ),
                probe(
                    format!("r1-release-{index}"),
                    WORKLOAD_UID,
                    &["release", &name],
                ),
            ]
        })
        .collect()
}

/// A trial of R1 shows the claim when the running process's lookup failed with `ENOENT` before
/// the host created the file, and found a file after.
pub fn judge_r1(results: &[ProbeResult], trials: usize) -> Tally {
    let _ = results;
    Tally {
        claimed: trials,
        trials: Vec::new(),
    }
}

/// R2's configurations, in the order they run.
pub const R2_CONFIGS: [&str; 3] = ["visible", "private", "unreachable"];

/// R2's probes: one guest-root staging per configuration.
pub fn r2_probes() -> Vec<Probe> {
    R2_CONFIGS
        .iter()
        .map(|config| probe(format!("r2-{config}"), ROOT_UID, &["stage", config]))
        .collect()
}

/// R2's verdict for each configuration. `visible` shows the claim when the mount succeeded and
/// the workload found the file; the other two when the mount succeeded and the workload's `stat`
/// failed with `ENOENT`.
pub fn judge_r2(results: &[ProbeResult]) -> Vec<(String, Verdict)> {
    let _ = results;
    Vec::new()
}

/// The tag of R3's share, which only R3's probes mount.
pub const R3_TAG: &str = "r3";
/// The file R3's holders hold, relative to R3's share.
pub const R3_FILE: &str = "data.txt";

/// R3's probes: per trial, guest root mounts the share, the workload holds a descriptor and a
/// mapping, guest root detaches the mount lazily, the workload looks the path up, and each holder
/// reads its bytes back.
pub fn r3_probes(trials: usize) -> Vec<Probe> {
    (0..trials)
        .flat_map(|index| {
            let target = format!("/mnt/r3/{index}");
            let file = format!("{target}/{R3_FILE}");
            let fd = format!("r3fd{index}");
            let map = format!("r3map{index}");
            [
                probe(
                    format!("r3-mount-{index}"),
                    ROOT_UID,
                    &["mount", R3_TAG, &target, "virtiofs"],
                ),
                probe(
                    format!("r3-hold-fd-{index}"),
                    WORKLOAD_UID,
                    &["hold-fd", &fd, &file],
                ),
                probe(
                    format!("r3-hold-map-{index}"),
                    WORKLOAD_UID,
                    &["hold-map", &map, &file],
                ),
                probe(
                    format!("r3-detach-{index}"),
                    ROOT_UID,
                    &["umount-detach", &target],
                ),
                probe(
                    format!("r3-lookup-{index}"),
                    WORKLOAD_UID,
                    &["lookup", &file],
                ),
                probe(
                    format!("r3-release-fd-{index}"),
                    WORKLOAD_UID,
                    &["release", &fd],
                ),
                probe(
                    format!("r3-release-map-{index}"),
                    WORKLOAD_UID,
                    &["release", &map],
                ),
            ]
        })
        .collect()
}

/// A trial of R3 shows the claim when the mount and the lazy unmount succeeded, the path then
/// failed with `ENOENT`, and both holders read `expected` back.
pub fn judge_r3(results: &[ProbeResult], trials: usize, expected: &Expected) -> Tally {
    let _ = (results, expected);
    Tally {
        claimed: trials,
        trials: Vec::new(),
    }
}

/// The file R4's holders hold, relative to the share.
pub const R4_FILE: &str = "opt/r4/data.txt";
/// The directory R4's thread works in, relative to the share.
pub const R4_DIR: &str = "opt/r4/dir";

/// R4's probes: per trial of the first part, a thread that unshares its descriptor table and
/// working directory holds the file and the directory, guest root scans `/proc`, and the thread
/// reads the file back; then per trial of the second, a descriptor in flight over `SCM_RIGHTS`
/// is scanned for and then received.
pub fn r4_probes(trials: usize) -> Vec<Probe> {
    let file = in_share(R4_FILE);
    let dir = in_share(R4_DIR);
    let threads = (0..trials).flat_map(|index| {
        let name = format!("t{index}");
        [
            probe(
                format!("r4a-hold-{index}"),
                WORKLOAD_UID,
                &["hold-thread", &name, &file, &dir],
            ),
            probe(format!("r4a-scan-{index}"), ROOT_UID, &["scan", &file, &dir]),
            probe(
                format!("r4a-release-{index}"),
                WORKLOAD_UID,
                &["release", &name],
            ),
        ]
    });
    let in_flight = (0..trials).flat_map(|index| {
        let name = format!("f{index}");
        [
            probe(
                format!("r4b-hold-{index}"),
                WORKLOAD_UID,
                &["hold-inflight", &name, &file],
            ),
            probe(format!("r4b-scan-{index}"), ROOT_UID, &["scan", &file]),
            probe(
                format!("r4b-release-{index}"),
                WORKLOAD_UID,
                &["release", &name],
            ),
        ]
    });
    threads.chain(in_flight).collect()
}

/// A trial of R4's first part shows the claim when the scan found the holder's file and
/// directory under the thread's task entry and nowhere under its process entry, and the thread
/// read `expected` back.
pub fn judge_r4_thread(results: &[ProbeResult], trials: usize, expected: &Expected) -> Tally {
    let _ = (results, expected);
    Tally {
        claimed: trials,
        trials: Vec::new(),
    }
}

/// A trial of R4's second part shows the claim when the scan found the file in no process's or
/// task's entry while its descriptor was in flight, and the received descriptor read `expected`.
pub fn judge_r4_in_flight(results: &[ProbeResult], trials: usize, expected: &Expected) -> Tally {
    let _ = (results, expected);
    Tally {
        claimed: trials,
        trials: Vec::new(),
    }
}

/// The eight name pairs measured as one name on this Mac's APFS data volume (PR #183's review):
/// a label, the spelling the host places, and the spelling the guest also looks up.
pub const NAME_PAIRS: [(&str, &str, &str); 8] = [
    ("cafe NFC / NFD", "caf\u{e9}", "cafe\u{301}"),
    ("sigma final / medial", "\u{3c3}\u{3b1}\u{3c2}", "\u{3c3}\u{3b1}\u{3c3}"),
    ("sharp s / ss", "stra\u{df}e", "strasse"),
    ("fi ligature / fi", "\u{fb01}le", "file"),
    ("Cafe NFC / CAFE NFD", "Caf\u{e9}", "CAFE\u{301}"),
    ("Kelvin sign / k", "\u{212a}", "k"),
    ("dotted capital I / i with dot", "\u{130}", "i\u{307}"),
    ("capital sharp s / sharp s", "\u{1e9e}", "\u{df}"),
];

/// Where the host places pair `index`'s file, relative to the share.
pub fn name_dir(index: usize) -> String {
    format!("names/{index}")
}

/// The name observation's probes: per pair, the workload stats both spellings and lists the
/// pair's directory.
pub fn name_probes() -> Vec<Probe> {
    NAME_PAIRS
        .iter()
        .enumerate()
        .flat_map(|(index, (_, placed, other))| {
            let dir = in_share(&name_dir(index));
            [
                probe(
                    format!("name-{index}-placed"),
                    WORKLOAD_UID,
                    &["stat", &format!("{dir}/{placed}")],
                ),
                probe(
                    format!("name-{index}-other"),
                    WORKLOAD_UID,
                    &["stat", &format!("{dir}/{other}")],
                ),
                probe(format!("name-{index}-list"), WORKLOAD_UID, &["list", &dir]),
            ]
        })
        .collect()
}

/// What the guest saw for each pair: `one name` when the other spelling found the placed file's
/// inode, `two names` when it failed with `ENOENT`, and anything else as seen.
pub fn observe_names(results: &[ProbeResult]) -> Vec<(String, String)> {
    let _ = results;
    Vec::new()
}

/// A copy of `lane-probe` that the setuid observation marks owned by root with mode 4755.
pub const SUID_TOOL: &str = "opt/suid/tool";

/// The setuid observation's probes: the workload runs the marked copy as a holder, whose ready
/// line reports the holder's effective uid, then releases it.
pub fn suid_probes() -> Vec<Probe> {
    vec![
        probe(
            "suid-hold",
            WORKLOAD_UID,
            &["hold-exec", "suid", &in_share(SUID_TOOL)],
        ),
        probe("suid-release", WORKLOAD_UID, &["release", "suid"]),
    ]
}

/// The value of a probe's result line, for a caller that records it.
pub fn result_of<'a>(results: &'a [ProbeResult], id: &str) -> Option<&'a Value> {
    results
        .iter()
        .find(|result| result.probe == id)
        .and_then(|result| result.result.as_ref())
}
