#![cfg(target_os = "macos")]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use common::gate::{
    GateLane, Shown, assert_decided, expected, gate_lane, record_lane, results, run_gate,
    test_commit, write_file, write_record,
};
use plasmosome_lanes::config::{Semantics, Share};
use plasmosome_lanes::gate::{self, R3_FILE, R3_TAG, R4_DIR, R4_FILE, Tally, Verdict};
use plasmosome_lanes::results::Record;
use plasmosome_lanes::steps::Step;
use serde_json::json;

const R1_TRIALS: usize = 80;
const R3_TRIALS: usize = 16;
const R4_TRIALS: usize = 6;
const LONG: Duration = Duration::from_secs(600);
const R3_DATA: &[u8] = b"held across a lazy unmount\n";
const R4_DATA: &[u8] = b"held by a thread or in flight\n";

fn plain(name: &str, probes: Vec<plasmosome_lanes::config::Probe>) -> GateLane {
    gate_lane(name, probes, Semantics::Complete, true, Vec::new())
}

fn trial_notes(row: &str, tally: &Tally) -> Vec<String> {
    tally
        .trials
        .iter()
        .map(|trial| format!("{row} trial {}: {:?}", trial.index, trial.outcome))
        .collect()
}

fn decided(row: &str, tally: &Tally) -> Verdict {
    let verdict = tally.verdict();
    eprintln!("{row}: {} of {} held", tally.held(), tally.claimed);
    assert_decided(row, &verdict);
    verdict
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn r1_a_running_process_finds_a_subtree_the_host_creates_in_its_share() {
    let gate = plain("r1-created-subtree", gate::r1_probes(R1_TRIALS));
    fs::create_dir_all(gate.share.join("opt")).expect("opt is created");
    let steps = (0..R1_TRIALS)
        .map(|index| {
            let relative = gate::r1_path(index);
            let path = gate.share.join(&relative);
            Step {
                after: gate::r1_watch(index),
                flag: gate.ctl.join(gate::r1_flag(index)),
                action: Box::new(move || {
                    fs::create_dir_all(path.parent().expect("a parent"))?;
                    fs::write(&path, b"x\n")?;
                    Ok(format!("created {relative}"))
                }),
            }
        })
        .collect();

    let run = run_gate(&gate, steps, LONG);

    let tally = gate::judge_r1(results(&run.run), R1_TRIALS);
    let verdict = decided("R1", &tally);
    let shown = Shown {
        verdicts: BTreeMap::from([("R1".to_string(), verdict)]),
        notes: trial_notes("R1", &tally),
        ..Shown::default()
    };
    record_lane(
        "r1-created-subtree",
        &gate,
        &[
            "share: an empty opt/ before the run",
            "per trial, after the watcher's ready line, the host creates opt/r1-<i>/bin/x and then writes the trial's flag",
        ],
        &run,
        shown,
    );
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn r2_mount_returns_0_while_the_workloads_stat_fails_enoent() {
    let gate = plain("r2-staging", gate::r2_probes());

    let run = run_gate(&gate, Vec::new(), common::guest::TIMEOUT);

    let verdicts: BTreeMap<String, Verdict> = gate::judge_r2(results(&run.run))
        .into_iter()
        .map(|(config, verdict)| (format!("R2 {config}"), verdict))
        .collect();
    for (row, verdict) in &verdicts {
        assert_decided(row, verdict);
    }
    record_lane(
        "r2-staging",
        &gate,
        &[
            "guest root makes /run/r2-<config>/opt/p, binds it onto itself and makes it shared",
            "a workload namespace is unshared from it: slave for visible and unreachable, private for private",
            "visible and private: the initial namespace binds /run/r2-source-<config>/p onto /run/r2-<config>/opt/p",
            "unreachable: the initial namespace binds /run/r2-hidden-unreachable over the source, then the workload namespace binds the source's p onto opt/p",
            "the workload's stat of opt/p/bin/x runs as uid 1000 inside the workload namespace",
        ],
        &run,
        Shown {
            verdicts,
            ..Shown::default()
        },
    );
    let reason = "027 §3.4 says four configurations and names two: private propagation, and a bind from a source the namespace cannot reach. PR #136's conversation names no other, so the other two could not be recovered and did not run";
    write_record(&Record {
        run: "r2-unrecovered".to_string(),
        configuration: json!({"reason": reason}),
        digests: BTreeMap::from([("test commit".to_string(), test_commit())]),
        verdicts: ["R2 third configuration", "R2 fourth configuration"]
            .into_iter()
            .map(|row| (row.to_string(), Verdict::NotRun(reason.to_string())))
            .collect(),
        observations: BTreeMap::new(),
        notes: Vec::new(),
        lines: Vec::new(),
    });
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn r3_a_descriptor_and_mapping_survive_a_lazy_unmount() {
    let mut gate = plain("r3-lazy-unmount", gate::r3_probes(R3_TRIALS));
    let source: PathBuf = gate.lane.root().join(R3_TAG);
    write_file(&source.join(R3_FILE), R3_DATA, 0o644);
    gate.lane.lane.shares.push(Share {
        tag: R3_TAG.to_string(),
        path: source,
        shm_size: 0,
        read_only: true,
        semantics: Semantics::Complete,
    });

    let run = run_gate(&gate, Vec::new(), LONG);

    let tally = gate::judge_r3(results(&run.run), R3_TRIALS, &expected(R3_DATA));
    let verdict = decided("R3", &tally);
    record_lane(
        "r3-lazy-unmount",
        &gate,
        &[
            "a read-only LINUX_COMPLETE share tagged r3 holding data.txt, which lane-init does not mount",
            "per trial, guest root mounts r3 at /mnt/r3/<i>, the workload holds a descriptor and a mapping of data.txt, guest root detaches the mount with umount2(MNT_DETACH), and the workload looks the path up and reads both back",
        ],
        &run,
        Shown {
            verdicts: BTreeMap::from([("R3".to_string(), verdict)]),
            notes: trial_notes("R3", &tally),
            ..Shown::default()
        },
    );
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn r4_a_threads_own_table_and_a_descriptor_in_flight() {
    let gate = plain("r4-tables", gate::r4_probes(R4_TRIALS));
    write_file(&gate.share.join(R4_FILE), R4_DATA, 0o644);
    fs::create_dir_all(gate.share.join(R4_DIR)).expect("the thread's directory is created");

    let run = run_gate(&gate, Vec::new(), LONG);

    let thread = gate::judge_r4_thread(results(&run.run), R4_TRIALS, &expected(R4_DATA));
    let in_flight = gate::judge_r4_in_flight(results(&run.run), R4_TRIALS, &expected(R4_DATA));
    let thread_verdict = decided("R4 thread", &thread);
    let in_flight_verdict = decided("R4 in flight", &in_flight);
    let mut notes = trial_notes("R4 thread", &thread);
    notes.extend(trial_notes("R4 in flight", &in_flight));
    record_lane(
        "r4-tables",
        &gate,
        &[
            "share: opt/r4/data.txt and an empty opt/r4/dir",
            "first part: a holder's thread calls unshare(CLONE_FS | CLONE_FILES), opens data.txt and enters dir, and guest root scans every /proc/<pid>/ and /proc/<pid>/task/<tid>/ for both by device and inode",
            "second part: a holder sends data.txt's descriptor over SCM_RIGHTS to its own socket and closes it, and guest root scans for the file",
        ],
        &run,
        Shown {
            verdicts: BTreeMap::from([
                ("R4 thread".to_string(), thread_verdict),
                ("R4 in flight".to_string(), in_flight_verdict),
            ]),
            notes,
            ..Shown::default()
        },
    );
}
