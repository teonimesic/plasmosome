mod comparison;
mod manifest;
mod normalization;
mod runs;

use serde_json::{Map, Value, json};

use super::{
    Finding, Lane, Manifest, ManifestRefusal, Observation, Reason, Run, Subject, Verdict, judge,
    load,
};

const REACH: [(&str, &str, &str); 9] = [
    ("refusal-host-1", "refusal", "host"),
    ("refusal-host-2", "refusal", "host"),
    ("refusal-host-3", "refusal", "host"),
    ("facility-1", "facility", "facility"),
    ("facility-2", "facility", "facility"),
    ("facility-3", "facility", "facility"),
    ("disturbance-path-1", "disturbance", "path"),
    ("disturbance-path-2", "disturbance", "path"),
    ("disturbance-path-3", "disturbance", "path"),
];

const PLAIN: [(&str, &str); 8] = [
    ("hint-mounts", "hint"),
    ("hint-cgroup", "hint"),
    ("hint-env", "hint"),
    ("hint-processes", "hint"),
    ("hint-self", "hint"),
    ("op-inodes", "operation"),
    ("op-times", "operation"),
    ("op-utimensat", "operation"),
];

fn manifest_json() -> Value {
    let mut probes: Vec<Value> = REACH
        .iter()
        .map(|(id, channel, kind)| {
            json!({"id": id, "channel": channel, "step": "after attach", "reach": kind, "workload_set": []})
        })
        .collect();
    probes.extend(PLAIN.iter().map(|(id, channel)| {
        let set: &[&str] = if *id == "op-utimensat" {
            &["mtime"]
        } else {
            &[]
        };
        json!({"id": id, "channel": channel, "step": "after attach", "workload_set": set})
    }));
    json!({
        "probes": probes,
        "rules": [
            {"probe": "hint-processes", "field": "pids", "kind": "pid"},
            {"probe": "hint-processes", "field": "leader", "kind": "pid"},
            {"probe": "hint-self", "field": "pid", "kind": "pid"},
            {"probe": "op-inodes", "field": "distinct", "kind": "inode"},
            {"probe": "op-inodes", "field": "linked", "kind": "inode"},
            {"probe": "op-times", "field": "times", "kind": "system_time"},
            {"probe": "op-utimensat", "field": "ctime", "kind": "system_time"}
        ],
        "allowances": [{"probe": "op-utimensat", "field": "mode", "cell_value": "0600"}],
        "neutral": {
            "mounts_probe": "hint-mounts",
            "cgroup_probe": "hint-cgroup",
            "mounts": [
                {"path": "/opt", "source": "none", "fstype": "virtiofs", "options": "ro"},
                {"path": "/usr/local/bin", "source": "none", "fstype": "virtiofs", "options": "ro"},
                {"path": "/workspace", "source": "workspace", "fstype": "fuse", "options": "rw"}
            ],
            "fuse_subtype": "passthrough",
            "cgroup_path": "/workload"
        }
    })
}

fn load_value(value: &Value) -> Result<Manifest, ManifestRefusal> {
    load(&value.to_string())
}

fn manifest() -> Manifest {
    load_value(&manifest_json()).expect("the fixture manifest loads")
}

fn ids() -> impl Iterator<Item = &'static str> {
    REACH
        .iter()
        .map(|(id, _, _)| *id)
        .chain(PLAIN.iter().map(|(id, _)| *id))
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().expect("an object").clone()
}

fn fields(lane: Lane, run: u64, probe: &str) -> Map<String, Value> {
    let seed = 10 * lane as u64 + run;
    object(match probe {
        "hint-mounts" => json!({
            "/opt": {"source": "none", "fstype": "virtiofs", "options": "ro"},
            "/usr/local/bin": {"source": "none", "fstype": "virtiofs", "options": "ro"},
            "/workspace": {"source": "workspace", "fstype": "fuse", "options": "rw", "subtype": "passthrough"}
        }),
        "hint-cgroup" => json!({"path": "/workload"}),
        "hint-env" => json!({"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/home/workload"}),
        "hint-processes" => json!({"pids": [1, 100 + seed, 200 + seed], "leader": 200 + seed}),
        "hint-self" => json!({"pid": 200 + seed}),
        "op-inodes" => {
            json!({"distinct": [1000 + seed, 2000 + seed], "linked": [3000 + seed, 3000 + seed]})
        }
        "op-times" => json!({"times": [10_000 + seed, 20_000 + seed]}),
        "op-utimensat" => json!({
            "mtime": 1_234_567_890,
            "ctime": 30_000 + seed,
            "mode": if lane == Lane::Cell { "0600" } else { "0644" }
        }),
        _ if lane == Lane::Open => json!({"outcome": "reached"}),
        _ => json!({"outcome": "name does not resolve"}),
    })
}

fn lane_run(lane: Lane, run: u64) -> Run {
    Run {
        lane,
        boot_id: format!("{lane:?}-{run}"),
        observations: ids()
            .map(|id| Observation {
                probe: id.to_string(),
                completed: true,
                failed_before_step: false,
                fields: fields(lane, run, id),
            })
            .collect(),
        transcript: Some(object(json!({
            "tool_calls": [{"tool": "bash", "input": "ls /opt"}],
            "exit_statuses": [0],
            "files_left_behind": ["/workspace/notes.txt"]
        }))),
    }
}

fn runs() -> Vec<Run> {
    Lane::ALL
        .into_iter()
        .flat_map(|lane| [lane_run(lane, 0), lane_run(lane, 1)])
        .collect()
}

fn observation<'a>(
    runs: &'a mut [Run],
    lane: Lane,
    run: usize,
    probe: &str,
) -> &'a mut Observation {
    runs[2 * lane as usize + run]
        .observations
        .iter_mut()
        .find(|candidate| candidate.probe == probe)
        .expect("the run observed that probe")
}

fn set(runs: &mut [Run], lane: Lane, probe: &str, field: &str, value: Value) {
    for run in 0..2 {
        observation(runs, lane, run, probe)
            .fields
            .insert(field.to_string(), value.clone());
    }
}

fn verdict(manifest: &Manifest, runs: &[Run]) -> Verdict {
    judge(manifest, runs).expect("the runs can be judged")
}

fn finding(probe: &str, reason: Reason) -> Finding {
    Finding {
        subject: Subject::Probe(probe.to_string()),
        reason,
    }
}

fn differs(probe: &str, field: &str) -> Finding {
    finding(
        probe,
        Reason::Differs {
            field: field.to_string(),
        },
    )
}

#[test]
fn runs_that_differ_only_in_ruled_values_and_the_allowed_mode_pass() {
    assert_eq!(verdict(&manifest(), &runs()), Verdict::Pass);
}
