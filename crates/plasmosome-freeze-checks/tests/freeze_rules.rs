use std::path::PathBuf;
use std::process::Command;

const CONTROLLER_CRATES: &[&str] = &["plasmosome-core", "plasmosome-backend", "plasmosome-ledger"];

const TESTKIT: &str = "plasmosome-testkit";

const FORBIDDEN_CRATE_FRAGMENTS: &[&str] = &[
    "libkrun",
    "krun",
    "smoltcp",
    "netstack",
    "hypervisor",
    "vmm",
    "vfio",
    "membrane",
    "egressd",
    "dnsd",
    "ak-vz",
    "ak-init",
    "ak-loop",
];

const FORBIDDEN_DIRECT_DEPENDENCIES: &[&str] = &[
    "libc",
    "nix",
    "rustix",
    "libkrun",
    "a3s-libkrun-sys",
    "netstack-smoltcp",
    "ak-netstack",
    "ak-vmm",
    "plasmosome-vmm",
    "ak-vz",
    "ak-egressd",
    "ak-dnsd",
    "ak-init",
    "plasmosome-membrane",
];

const SHARED_MEMORY_PATTERNS: &[&str] = &[
    "Arc<",
    "Rc<",
    "Mutex",
    "RwLock",
    "UnsafeCell",
    "thread_local",
    "lazy_static",
    "once_cell",
    "static mut",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the checks crate sits two levels below the workspace root")
        .to_path_buf()
}

fn cargo() -> Command {
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
    command.current_dir(workspace_root());
    command
}

#[test]
fn controller_crates_have_no_dependency_path_to_a_vmm_or_netstack_crate() {
    for package in CONTROLLER_CRATES {
        let output = cargo()
            .args([
                "tree",
                "--locked",
                "-p",
                package,
                "--edges",
                "normal,build,dev",
                "--prefix",
                "none",
            ])
            .output()
            .expect("cargo tree runs");
        assert!(
            output.status.success(),
            "cargo tree -p {package} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let graph = String::from_utf8_lossy(&output.stdout);
        for line in graph.lines() {
            for forbidden in FORBIDDEN_CRATE_FRAGMENTS {
                assert!(
                    !line.contains(forbidden),
                    "86 §4 rule 1 broken: `{package}` depends on `{line}`, which names the forbidden VMM/netstack/membrane/broker fragment `{forbidden}`; the controller must compile and test with no dependency path to any of them"
                );
            }
        }
    }
}

#[test]
fn controller_crates_declare_no_fork_or_socketpair_plumbing_dependency() {
    for package in CONTROLLER_CRATES {
        let manifest = std::fs::read_to_string(
            workspace_root()
                .join("crates")
                .join(package)
                .join("Cargo.toml"),
        )
        .expect("the crate manifest is readable");
        let declared = declared_dependencies(&manifest);
        for dependency in &declared {
            for forbidden in FORBIDDEN_DIRECT_DEPENDENCIES {
                assert!(
                    dependency != *forbidden,
                    "86 §4 rule 1 broken: `{package}` directly depends on `{dependency}`; fork/socketpair plumbing and supervisor/broker crates must never enter the controller's manifest"
                );
            }
        }
    }
}

fn declared_dependencies(manifest: &str) -> Vec<String> {
    declared_in(manifest, "[dependencies]")
}

fn declared_in(manifest: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut names = Vec::new();
    for line in manifest.lines() {
        if line.starts_with('[') {
            in_section = line == section;
            continue;
        }
        if in_section && let Some(name) = line.split('=').next() {
            let name = name.trim();
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    names
}

fn workspace_members() -> Vec<String> {
    let manifest = std::fs::read_to_string(workspace_root().join("Cargo.toml"))
        .expect("the workspace manifest is readable");
    let mut in_members = false;
    let mut members = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line == "members = [" {
            in_members = true;
            continue;
        }
        if in_members {
            if line == "]" {
                break;
            }
            let path = line.trim_matches(|c: char| c == ',' || c == '"');
            let name = path.rsplit('/').next().unwrap_or(path);
            members.push(name.to_string());
        }
    }
    assert!(
        members.contains(&TESTKIT.to_string()),
        "the workspace manifest no longer lists its members one per line: {members:?}"
    );
    members
}

#[test]
fn testkit_is_dev_only() {
    for member in workspace_members() {
        if member == TESTKIT {
            continue;
        }
        let output = cargo()
            .args([
                "tree",
                "--locked",
                "-p",
                &member,
                "--edges",
                "normal,build",
                "--prefix",
                "none",
                "--target",
                "all",
            ])
            .output()
            .expect("cargo tree runs");
        assert!(
            output.status.success(),
            "cargo tree -p {member} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let graph = String::from_utf8_lossy(&output.stdout);
        assert!(
            !graph.lines().any(|line| line.starts_with(TESTKIT)),
            "`{member}` has a non-dev dependency path to `{TESTKIT}`; the testkit is test support and reaches a kernel crate only through `[dev-dependencies]`, or it ships"
        );
    }
}

#[test]
fn controller_wire_state_shares_no_memory_across_the_seam() {
    let wire_sources = [
        "crates/plasmosome-core/src/state.rs",
        "crates/plasmosome-core/src/reconciler.rs",
        "crates/plasmosome-core/src/manifest.rs",
        "crates/plasmosome-backend/src/backend.rs",
        "crates/plasmosome-backend/src/universe.rs",
        "crates/plasmosome-ledger/src/lib.rs",
    ];
    for relative in wire_sources {
        let source = std::fs::read_to_string(workspace_root().join(relative))
            .expect("the wire module is readable");
        for pattern in SHARED_MEMORY_PATTERNS {
            assert!(
                !source.contains(pattern),
                "86 §4 rule 2 broken: `{relative}` uses `{pattern}`; controller⇄supervisor state moves only as serde types, never as shared memory"
            );
        }
    }
}

#[test]
fn every_seam_wire_type_is_serde_in_both_directions() {
    use plasmosome_backend::{
        Capability, Diff, DrainSpec, Grant, GrantKind, Handle, LedgerEntry, OsObject, OsState,
        PluginId, ResidueReport, RevokePolicy, UniverseClass, UniverseOp, UniverseRemoval,
    };
    use plasmosome_core::{
        CellId, CellRecord, CellStatus, ControllerState, GenomeName, InstanceName, InstanceRecord,
        MockMode, PlasmidRecord,
    };
    use plasmosome_ledger::{Effect, LogRecord};

    fn wire_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
    wire_serde::<Handle>();
    wire_serde::<GrantKind>();
    wire_serde::<Capability>();
    wire_serde::<Grant>();
    wire_serde::<LedgerEntry>();
    wire_serde::<RevokePolicy>();
    wire_serde::<DrainSpec>();
    wire_serde::<PluginId>();
    wire_serde::<UniverseClass>();
    wire_serde::<OsObject>();
    wire_serde::<OsState>();
    wire_serde::<Diff>();
    wire_serde::<UniverseOp>();
    wire_serde::<UniverseRemoval>();
    wire_serde::<ResidueReport>();
    wire_serde::<Effect>();
    wire_serde::<LogRecord>();
    wire_serde::<ControllerState>();
    wire_serde::<InstanceRecord>();
    wire_serde::<InstanceName>();
    wire_serde::<CellRecord>();
    wire_serde::<CellId>();
    wire_serde::<CellStatus>();
    wire_serde::<GenomeName>();
    wire_serde::<PlasmidRecord>();
    wire_serde::<MockMode>();
}
