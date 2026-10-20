#![allow(dead_code)]

#[cfg(target_os = "macos")]
pub mod gate;
#[cfg(target_os = "macos")]
pub mod guest;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use plasmosome_lanes::config::{
    Confinement, GuestConfig, GuestMount, LaneConfig, MountOption, Probe, Semantics, Share,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

pub const LIBKRUN: &str = "lib/libkrun.1.19.4.dylib";
pub const KERNEL: &str = "boot/kernel";

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn write_file(dir: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().expect("a fixture file has a parent"))
        .expect("the fixture directory is created");
    fs::write(&path, bytes).expect("the fixture file is written");
    path
}

pub fn write_sums(dir: &Path, lines: &[(String, &str)]) {
    let text: String = lines
        .iter()
        .map(|(digest, path)| format!("{digest}  {path}\n"))
        .collect();
    fs::write(dir.join("SHA256SUMS"), text).expect("SHA256SUMS is written");
}

pub const BUNDLE_FILES: &[(&str, &[u8])] = &[
    (LIBKRUN, b"not really a dylib"),
    (KERNEL, b"not really a kernel"),
    ("boot/initramfs", b"not really an initramfs"),
    ("base/rootfs.raw", b"not really a disk"),
];

/// A bundle directory whose name and manifest chain to its `SHA256SUMS`, as a real bundle's do.
pub struct BundleFixture {
    _parent: TempDir,
    root: PathBuf,
    path: PathBuf,
    pub id: String,
}

impl BundleFixture {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn parent(&self) -> &Path {
        &self.root
    }

    pub fn write_manifest(&self, manifest: &Value) {
        fs::write(
            self.path.join("manifest.json"),
            serde_json::to_vec(manifest).expect("the manifest serializes"),
        )
        .expect("manifest.json is written");
    }

    pub fn sums_digest(&self) -> String {
        sha256(&fs::read(self.path.join("SHA256SUMS")).expect("SHA256SUMS is readable"))
    }

    /// Rewrite the manifest for the current `SHA256SUMS` and rename the directory to match.
    pub fn reseal(&mut self) {
        let manifest = json!({
            "bundle_format": "plasmosome-runtime-bundle/1",
            "sha256sums_sha256": self.sums_digest(),
        });
        self.write_manifest(&manifest);
        self.rename_to_manifest();
    }

    /// Rename the directory to the first 16 hex digits of its manifest's SHA-256.
    pub fn rename_to_manifest(&mut self) {
        let digest =
            sha256(&fs::read(self.path.join("manifest.json")).expect("manifest.json is readable"));
        let id = digest[..16].to_string();
        let renamed = self.root.join(&id);
        fs::rename(&self.path, &renamed).expect("the bundle directory is renamed");
        self.path = renamed;
        self.id = id;
    }
}

pub fn bundle_fixture() -> BundleFixture {
    let parent = tempfile::tempdir().expect("a bundle fixture directory");
    let root = fs::canonicalize(parent.path()).expect("the fixture directory resolves");
    let path = root.join("unsealed");
    let mut lines = Vec::new();
    for (relative, bytes) in BUNDLE_FILES {
        write_file(&path, relative, bytes);
        lines.push((sha256(bytes), *relative));
    }
    write_sums(&path, &lines);
    let mut fixture = BundleFixture {
        _parent: parent,
        root,
        path,
        id: String::new(),
    };
    fixture.reseal();
    fixture
}

pub fn lane_json(root: &Path) -> Value {
    json!({
        "run_id": "run-1",
        "bundle": root.join("bundle"),
        "initramfs": root.join("initramfs.cpio"),
        "root_image": root.join("root.ext4"),
        "vcpus": 1,
        "memory_mib": 512,
        "console": root.join("console.log"),
        "shares": [{
            "tag": "lanes0",
            "path": root.join("share"),
            "shm_size": 0,
            "read_only": true,
            "semantics": "complete"
        }],
        "confinement": null
    })
}

pub fn guest_json() -> Value {
    json!({
        "run_id": "run-1",
        "mounts": [{"tag": "lanes0", "path": "/mnt/share", "options": ["ro", "nodev"]}],
        "probes": [
            {"id": "uname", "args": ["uname"]},
            {"id": "root-id", "uid": 0, "args": ["id"]}
        ]
    })
}

pub fn lane_config(root: &Path, run_id: &str) -> LaneConfig {
    LaneConfig {
        run_id: run_id.to_string(),
        bundle: root.join("bundle"),
        initramfs: root.join("initramfs.cpio"),
        root_image: root.join("root.ext4"),
        vcpus: 1,
        memory_mib: 512,
        console: root.join("console.log"),
        shares: vec![Share {
            tag: "lanes0".to_string(),
            path: root.join("share"),
            shm_size: 0,
            read_only: true,
            semantics: Semantics::Complete,
        }],
        confinement: None,
    }
}

pub fn confinement(root: &Path) -> Confinement {
    Confinement {
        profile: root.join("profile.sb"),
        allowed: root.join("allowed"),
        outside: root.join("outside"),
    }
}

pub fn guest_config(run_id: &str, probe_ids: &[&str]) -> GuestConfig {
    GuestConfig {
        run_id: run_id.to_string(),
        mounts: vec![GuestMount {
            tag: "lanes0".to_string(),
            path: PathBuf::from("/mnt/share"),
            options: vec![MountOption::Ro],
        }],
        probes: probe_ids
            .iter()
            .map(|id| Probe {
                id: id.to_string(),
                uid: 1000,
                args: vec!["id".to_string()],
            })
            .collect(),
    }
}

const LAUNCHER: &str = "#!/bin/sh\nexec /bin/sh \"$0.sh\"\n";

fn shared_launcher() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plasmosome-lanes-launcher");
        fs::create_dir_all(&dir).expect("the shared launcher directory is created");
        let path = dir.join("launcher");
        if fs::read_to_string(&path).ok().as_deref() != Some(LAUNCHER) {
            let staged = dir.join(format!("launcher.{}", std::process::id()));
            fs::write(&staged, LAUNCHER).expect("the shared launcher is written");
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
                .expect("the shared launcher is made executable");
            fs::rename(&staged, &path).expect("the shared launcher is put in place");
        }
        fs::write(dir.join("launcher.sh"), ":\n").expect("the warm-up body is written");
        let warm = Command::new(&path)
            .env_clear()
            .stdin(Stdio::null())
            .status()
            .expect("the shared launcher runs once");
        assert!(
            warm.success(),
            "the shared launcher's warm-up run exited {warm}"
        );
        path
    })
}

/// A launcher at `dir/name` that runs `body` with `/bin/sh`. It is a link to one shared script,
/// run once before any test uses it, because macOS assesses each new executable file the first
/// time it runs, and that took seconds per file on a busy machine.
pub fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(dir.join(format!("{name}.sh")), format!("{body}\n"))
        .expect("the launcher body is written");
    symlink(shared_launcher(), &path).expect("the launcher link is made");
    path
}

pub fn result_json(probe: &str, uid: u32) -> String {
    json!({"probe": probe, "uid": uid, "exit": 0, "result": {"probe": "id", "ok": true}})
        .to_string()
}
