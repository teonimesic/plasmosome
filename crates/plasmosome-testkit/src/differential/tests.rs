mod manifest;

use serde_json::{Value, json};

use super::{Manifest, ManifestRefusal, load};

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
