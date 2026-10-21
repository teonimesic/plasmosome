use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use plasmosome_lanes::bundle::{PINNED_BUNDLE_ID, verify_bundle};
use plasmosome_lanes::config::{GuestConfig, LaneConfig, Probe};
use plasmosome_lanes::console::ProbeResult;
use plasmosome_lanes::cpio::lane_initramfs;
use plasmosome_lanes::run::{LaneRun, Outcome, run_lane};
use serde_json::Value;
use tempfile::TempDir;

pub const BUNDLE: &str = "/usr/local/var/plasmosome/runtime/bundles/eed141b574337d99";
pub const TIMEOUT: Duration = Duration::from_secs(90);
const EM_AARCH64: u16 = 183;

pub fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn bundle_dir() -> PathBuf {
    let dir = std::env::var_os("LANES_BUNDLE").map_or_else(|| PathBuf::from(BUNDLE), PathBuf::from);
    verify_bundle(&dir, PINNED_BUNDLE_ID).unwrap_or_else(|error| {
        panic!(
            "this test needs the pinned bundle at {}: {error}",
            dir.display()
        )
    });
    dir
}

pub fn guest_binary_path(name: &str) -> PathBuf {
    std::env::var_os("LANES_GUEST_DIR")
        .map_or_else(
            || manifest_dir().join("../../target/aarch64-unknown-linux-musl/release"),
            PathBuf::from,
        )
        .join(name)
}

pub fn guest_binary(name: &str) -> Vec<u8> {
    let path = guest_binary_path(name);
    let bytes = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "this test needs the guest binary {} ({error}); build it as the crate README says",
            path.display()
        )
    });
    assert_eq!(&bytes[..4], b"\x7fELF", "{} is not ELF", path.display());
    assert_eq!(bytes[4], 2, "{} is not a 64-bit ELF", path.display());
    assert_eq!(
        u16::from_le_bytes([bytes[18], bytes[19]]),
        EM_AARCH64,
        "{} is not an aarch64 executable",
        path.display()
    );
    bytes
}

pub fn signed_vmm() -> &'static Path {
    static SIGNED: OnceLock<PathBuf> = OnceLock::new();
    SIGNED.get_or_init(|| {
        let vmm = PathBuf::from(env!("CARGO_BIN_EXE_lane-vmm"));
        let output = Command::new("codesign")
            .args(["--sign", "-", "--force", "--entitlements"])
            .arg(manifest_dir().join("lane-vmm.entitlements"))
            .arg(&vmm)
            .output()
            .expect("codesign runs");
        assert!(
            output.status.success(),
            "ad hoc signing with the hypervisor entitlement failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        vmm
    })
}

pub struct Lane {
    pub dir: TempDir,
    pub lane: LaneConfig,
    pub guest: GuestConfig,
}

impl Lane {
    /// The lane's own scratch directory, which holds its console, initramfs and root image.
    pub fn root(&self) -> PathBuf {
        self.lane
            .console
            .parent()
            .expect("a lane directory")
            .to_path_buf()
    }
}

pub fn probe(id: &str, uid: u32, args: &[&str]) -> Probe {
    Probe {
        id: id.to_string(),
        uid,
        args: args.iter().map(|arg| arg.to_string()).collect(),
    }
}

pub fn lane(run_id: &str, probes: Vec<Probe>) -> Lane {
    let dir = tempfile::tempdir().expect("a scratch directory for the lane");
    let root = fs::canonicalize(dir.path()).expect("the scratch directory resolves");
    let bundle = bundle_dir();
    let guest = GuestConfig {
        run_id: run_id.to_string(),
        mounts: Vec::new(),
        probes,
    };
    let root_image = root.join("rootfs.raw");
    let mut from =
        fs::File::open(bundle.join("base/rootfs.raw")).expect("the bundle's image opens");
    let mut to = fs::File::create(&root_image).expect("the lane's root image is created");
    std::io::copy(&mut from, &mut to).expect("the root image is copied without its flags");
    let lane = LaneConfig {
        run_id: run_id.to_string(),
        bundle: bundle.clone(),
        initramfs: root.join("initramfs.cpio"),
        root_image,
        vcpus: 1,
        memory_mib: 512,
        console: root.join("console.log"),
        shares: Vec::new(),
        confinement: None,
    };
    Lane { dir, lane, guest }
}

pub fn write_initramfs(lane: &Lane) {
    let initramfs = lane_initramfs(
        &guest_binary("lane-init"),
        &guest_binary("lane-probe"),
        &lane.guest,
    )
    .expect("the initramfs is written");
    fs::write(&lane.lane.initramfs, initramfs).expect("the initramfs is saved");
}

pub fn report(run: &LaneRun) {
    eprintln!(
        "--- lane-vmm stderr ---\n{}--- console ---\n{}--- outcome: {:?}, status {:?}",
        run.stderr, run.console, run.outcome, run.status
    );
}

pub fn boot(lane: &Lane) -> LaneRun {
    write_initramfs(lane);
    let run = run_lane(signed_vmm(), &lane.lane, &lane.guest, TIMEOUT).expect("lane-vmm starts");
    report(&run);
    run
}

pub fn finished(run: &LaneRun) -> &[ProbeResult] {
    match &run.outcome {
        Outcome::Finished(results) => results,
        other => panic!("the lane did not finish: {other:?}"),
    }
}

pub fn line(results: &[ProbeResult], id: &str) -> Value {
    results
        .iter()
        .find(|result| result.probe == id)
        .and_then(|result| result.result.clone())
        .unwrap_or_else(|| panic!("no result for probe `{id}`"))
}
