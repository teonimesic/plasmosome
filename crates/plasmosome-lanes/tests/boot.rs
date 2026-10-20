#![cfg(target_os = "macos")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use plasmosome_lanes::bundle::verify_bundle;
use plasmosome_lanes::config::{
    Confinement, GuestConfig, GuestMount, LaneConfig, Probe, Semantics, Share,
};
use plasmosome_lanes::console::ProbeResult;
use plasmosome_lanes::cpio::lane_initramfs;
use plasmosome_lanes::run::{LaneRun, Outcome, run_lane};
use serde_json::Value;
use tempfile::TempDir;

const BUNDLE: &str = "/usr/local/var/plasmosome/runtime/bundles/eed141b574337d99";
const BUNDLE_KERNEL_RELEASE: &str = "6.12.91";
const TIMEOUT: Duration = Duration::from_secs(90);
const EM_AARCH64: u16 = 183;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bundle_dir() -> PathBuf {
    let dir = std::env::var_os("LANES_BUNDLE").map_or_else(|| PathBuf::from(BUNDLE), PathBuf::from);
    verify_bundle(&dir).unwrap_or_else(|error| {
        panic!(
            "this test needs the pinned bundle at {}: {error}",
            dir.display()
        )
    });
    dir
}

fn guest_binary(name: &str) -> Vec<u8> {
    let dir = std::env::var_os("LANES_GUEST_DIR").map_or_else(
        || manifest_dir().join("../../target/aarch64-unknown-linux-musl/release"),
        PathBuf::from,
    );
    let path = dir.join(name);
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

fn signed_vmm() -> &'static Path {
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

struct Lane {
    dir: TempDir,
    lane: LaneConfig,
    guest: GuestConfig,
}

fn probe(id: &str, uid: u32, args: &[&str]) -> Probe {
    Probe {
        id: id.to_string(),
        uid,
        args: args.iter().map(|arg| arg.to_string()).collect(),
    }
}

fn lane(run_id: &str, probes: Vec<Probe>) -> Lane {
    let dir = tempfile::tempdir().expect("a scratch directory for the lane");
    let root = fs::canonicalize(dir.path()).expect("the scratch directory resolves");
    let bundle = bundle_dir();
    let guest = GuestConfig {
        run_id: run_id.to_string(),
        mounts: Vec::new(),
        probes,
    };
    let lane = LaneConfig {
        run_id: run_id.to_string(),
        bundle: bundle.clone(),
        initramfs: root.join("initramfs.cpio"),
        root_image: bundle.join("base/rootfs.raw"),
        vcpus: 1,
        memory_mib: 512,
        console: root.join("console.log"),
        shares: Vec::new(),
        confinement: None,
    };
    Lane { dir, lane, guest }
}

fn boot(lane: &Lane) -> LaneRun {
    let initramfs = lane_initramfs(
        &guest_binary("lane-init"),
        &guest_binary("lane-probe"),
        &lane.guest,
    )
    .expect("the initramfs is written");
    fs::write(&lane.lane.initramfs, initramfs).expect("the initramfs is saved");
    let run = run_lane(signed_vmm(), &lane.lane, &lane.guest, TIMEOUT).expect("lane-vmm starts");
    eprintln!(
        "--- lane-vmm stderr ---\n{}--- console ---\n{}--- outcome: {:?}, status {:?}",
        run.stderr, run.console, run.outcome, run.status
    );
    run
}

fn finished(run: &LaneRun) -> &[ProbeResult] {
    match &run.outcome {
        Outcome::Finished(results) => results,
        other => panic!("the lane did not finish: {other:?}"),
    }
}

fn line(results: &[ProbeResult], id: &str) -> Value {
    results
        .iter()
        .find(|result| result.probe == id)
        .and_then(|result| result.result.clone())
        .unwrap_or_else(|| panic!("no result for probe `{id}`"))
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn the_bundle_kernel_boots_lane_init_and_lists_virtiofs() {
    let lane = lane(
        "boot-a",
        vec![
            probe("uname", 1000, &["uname"]),
            probe("filesystems", 1000, &["filesystems"]),
        ],
    );

    let run = boot(&lane);

    let results = finished(&run);
    let uname = line(results, "uname");
    assert_eq!(uname["ok"], true, "{uname}");
    assert_eq!(uname["sysname"], "Linux");
    assert_eq!(uname["release"], BUNDLE_KERNEL_RELEASE);
    assert_eq!(uname["machine"], "aarch64");
    let filesystems = line(results, "filesystems");
    let names = filesystems["names"].as_array().expect("a name list");
    assert!(
        names.iter().any(|name| name == "virtiofs"),
        "/proc/filesystems lists no virtiofs: {filesystems}"
    );
    assert!(lane.dir.path().exists());
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn a_read_only_complete_share_is_mounted_at_its_path_and_lists_the_hosts_file() {
    let mut lane = lane(
        "boot-b",
        vec![
            probe("list", 1000, &["list", "/mnt/share"]),
            probe("mountinfo", 1000, &["mountinfo"]),
            probe("create-as-root", 0, &["create", "/mnt/share/from-guest"]),
            probe("write-as-root", 0, &["write", "/mnt/share/hello.txt"]),
        ],
    );
    let share = lane.lane.console.with_file_name("share");
    common::write_file(&share, "hello.txt", b"placed by the host\n");
    lane.lane.shares.push(Share {
        tag: "lanes0".to_string(),
        path: share.clone(),
        shm_size: 0,
        read_only: true,
        semantics: Semantics::Complete,
    });
    lane.guest.mounts.push(GuestMount {
        tag: "lanes0".to_string(),
        path: PathBuf::from("/mnt/share"),
        options: Vec::new(),
    });

    let run = boot(&lane);

    let results = finished(&run);
    let list = line(results, "list");
    assert_eq!(list["ok"], true, "{list}");
    assert_eq!(list["names"], serde_json::json!(["hello.txt"]));
    let mountinfo = line(results, "mountinfo");
    let mounts = mountinfo["mounts"].as_array().expect("a mount list");
    let share_mount = mounts
        .iter()
        .find(|mount| mount["mount_point"] == "/mnt/share")
        .unwrap_or_else(|| panic!("nothing is mounted at /mnt/share: {mountinfo}"));
    assert_eq!(share_mount["fstype"], "virtiofs");
    assert_eq!(share_mount["source"], "lanes0");
    for id in ["create-as-root", "write-as-root"] {
        let refused = line(results, id);
        assert_eq!(
            refused["ok"], false,
            "{id} changed the read-only share: {refused}"
        );
        assert_eq!(refused["errno"], "EROFS", "{refused}");
    }
    assert!(
        !share.join("from-guest").exists(),
        "the guest created a file in the read-only share"
    );
    assert_eq!(
        fs::read(share.join("hello.txt")).expect("the host file is still there"),
        b"placed by the host\n"
    );
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn a_probe_configured_as_uid_1000_reports_uid_1000_and_one_as_0_reports_0() {
    let lane = lane(
        "boot-c",
        vec![probe("workload", 1000, &["id"]), probe("root", 0, &["id"])],
    );

    let run = boot(&lane);

    let results = finished(&run);
    let workload = line(results, "workload");
    assert_eq!(
        (&workload["uid"], &workload["euid"], &workload["gid"]),
        (&Value::from(1000), &Value::from(1000), &Value::from(1000)),
        "{workload}"
    );
    assert_eq!(workload["groups"], serde_json::json!([]));
    let root = line(results, "root");
    assert_eq!(root["uid"], 0, "{root}");
    assert_eq!(results[0].uid, 1000);
    assert_eq!(results[1].uid, 0);
}

fn confined_lane(run_id: &str, allow_writes_outside: bool) -> Lane {
    let mut lane = lane(run_id, vec![probe("uname", 1000, &["uname"])]);
    let root = lane
        .lane
        .console
        .parent()
        .expect("a lane directory")
        .to_path_buf();
    let allowed = root.join("allowed");
    let outside = root.join("outside");
    common::write_file(&allowed, "placed", b"placed\n");
    common::write_file(&outside, "secret", b"secret\n");
    let writes = if allow_writes_outside {
        format!("(deny file-write* (subpath \"{}\"))", allowed.display())
    } else {
        format!(
            "(deny file-write*)\n(allow file-write* (literal \"{}\"))",
            lane.lane.console.display()
        )
    };
    let profile = format!(
        "(version 1)\n(allow default)\n{writes}\n(deny file-read* (subpath \"{}\"))\n",
        outside.display()
    );
    let profile_path = root.join("lane.sb");
    fs::write(&profile_path, profile).expect("the profile is saved");
    lane.lane.confinement = Some(Confinement {
        profile: profile_path,
        allowed,
        outside,
    });
    lane
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn the_sbpl_self_check_passes_with_the_profile_and_the_confined_lane_boots() {
    let lane = confined_lane("boot-d", false);

    let run = boot(&lane);

    let results = finished(&run);
    assert_eq!(line(results, "uname")["ok"], true);
    for check in [
        "read inside",
        "write inside",
        "read outside",
        "write outside",
    ] {
        assert!(
            run.stderr
                .lines()
                .any(|row| row.contains(&format!("self-check {check}:")) && row.ends_with("holds")),
            "the self-check line for `{check}` is missing or fails: {}",
            run.stderr
        );
    }
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn the_sbpl_self_check_exits_70_when_the_profile_allows_writes_outside_the_directory() {
    let lane = confined_lane("boot-e", true);

    let run = boot(&lane);

    assert_eq!(run.status.and_then(|status| status.code()), Some(70));
    assert!(
        matches!(run.outcome, Outcome::NotRun(_)),
        "{:?}",
        run.outcome
    );
    assert_eq!(run.console, "", "the VM must not have started");
    assert!(
        run.stderr
            .lines()
            .any(|row| row.contains("self-check write outside:") && row.ends_with("FAILS")),
        "{}",
        run.stderr
    );
}

#[test]
#[ignore = "needs the pinned bundle and the hypervisor entitlement"]
fn a_held_descriptor_mapping_and_executable_are_read_back_on_release() {
    let mut lane = lane(
        "boot-f",
        vec![
            probe("hold-fd", 1000, &["hold-fd", "fd", "/mnt/share/hello.txt"]),
            probe(
                "hold-map",
                1000,
                &["hold-map", "map", "/mnt/share/hello.txt"],
            ),
            probe("hold-exec", 1000, &["hold-exec", "exec", "/mnt/share/tool"]),
            probe("release-fd", 1000, &["release", "fd"]),
            probe("release-map", 1000, &["release", "map"]),
            probe("release-exec", 1000, &["release", "exec"]),
            probe("release-none", 1000, &["release", "none"]),
        ],
    );
    let share = lane.lane.console.with_file_name("share");
    let placed = b"placed by the host\n";
    common::write_file(&share, "hello.txt", placed);
    let tool_bytes = guest_binary("lane-probe");
    let tool = common::write_file(&share, "tool", &tool_bytes);
    fs::set_permissions(&tool, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("the tool is made executable");
    lane.lane.shares.push(Share {
        tag: "lanes0".to_string(),
        path: share,
        shm_size: 0,
        read_only: true,
        semantics: Semantics::Complete,
    });
    lane.guest.mounts.push(GuestMount {
        tag: "lanes0".to_string(),
        path: PathBuf::from("/mnt/share"),
        options: Vec::new(),
    });

    let run = boot(&lane);

    let results = finished(&run);
    for id in ["hold-fd", "hold-map", "hold-exec"] {
        assert_eq!(line(results, id)["ok"], true, "{}", line(results, id));
    }
    for (id, bytes) in [
        ("release-fd", placed.as_slice()),
        ("release-map", placed.as_slice()),
        ("release-exec", tool_bytes.as_slice()),
    ] {
        let released = line(results, id);
        assert_eq!(released["ok"], true, "{released}");
        assert_eq!(released["sha256"], common::sha256(bytes), "{released}");
        assert_eq!(released["bytes"], bytes.len(), "{released}");
    }
    assert_eq!(line(results, "release-none")["errno"], "ENOENT");
}
