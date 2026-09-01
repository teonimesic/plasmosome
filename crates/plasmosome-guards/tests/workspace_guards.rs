use std::collections::BTreeMap;
use std::process::Command;

use plasmosome_guards::workspace_root;

const TESTKIT: &str = "plasmosome-testkit";

const HELD_NAMES: &[&str] = &["plasmosome", "plasmid"];

const HELD_REGISTRIES: &[&str] = &["crates-io"];

fn cargo() -> Command {
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
    command.current_dir(workspace_root());
    command
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

fn workspace_packages() -> Vec<serde_json::Value> {
    let output = cargo()
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata emits JSON");
    metadata["packages"]
        .as_array()
        .expect("cargo metadata reports the workspace packages")
        .clone()
}

fn binary_targets(package: &serde_json::Value) -> Vec<String> {
    package["targets"]
        .as_array()
        .expect("a package reports its targets")
        .iter()
        .filter(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .map(|target| {
            target["name"]
                .as_str()
                .expect("a target has a name")
                .to_string()
        })
        .collect()
}

fn package_name(package: &serde_json::Value) -> String {
    package["name"]
        .as_str()
        .expect("a package has a name")
        .to_string()
}

#[test]
fn only_the_held_names_are_publishable_to_a_registry() {
    let mut reported = Vec::new();
    for package in workspace_packages() {
        let name = package_name(&package);
        let registries = package["publish"].as_array().unwrap_or_else(|| {
            panic!(
                "`{name}` leaves `publish` unset, and `cargo metadata` reports an unset field and `publish = true` identically as null, so this rule cannot tell the two apart; every member of this workspace says where it may go explicitly — `publish = false`, or `publish = {HELD_REGISTRIES:?}` for a name this project holds"
            )
        });
        let registries: Vec<&str> = registries
            .iter()
            .map(|registry| registry.as_str().expect("a registry name is a string"))
            .collect();
        if HELD_NAMES.contains(&name.as_str()) {
            assert_eq!(
                registries, HELD_REGISTRIES,
                "`{name}` is a name this project holds on crates.io, so it carries `publish = {HELD_REGISTRIES:?}` and reaches that registry and no other; it currently says {registries:?}, and giving up a public name claim is a deliberate edit of `HELD_NAMES` rather than a manifest quietly closing itself"
            );
        } else {
            assert!(
                registries.is_empty(),
                "`{name}` may be published to {registries:?}; only {HELD_NAMES:?} are claimed on a registry, and releasing anything else from this workspace is a deliberate act that names it here first"
            );
        }
        reported.push(name);
    }

    reported.sort();
    for held in HELD_NAMES {
        assert!(
            reported.iter().any(|name| name.as_str() == *held),
            "`{held}` is on the publish allowlist and is not a package in this workspace; the counts below still agree without it, so an entry naming a crate that was renamed or removed would sit here unnoticed and hand its exemption to whatever takes the name next — it reported {reported:?}"
        );
    }

    let listed = workspace_members().len();
    assert_eq!(
        reported.len(),
        listed,
        "the workspace manifest lists {listed} members but `cargo metadata` reported {}, so this rule cannot claim to have checked them all; it checked {reported:?}",
        reported.len()
    );
}

fn binary_name_collisions(packages: &[serde_json::Value]) -> Vec<String> {
    let names: Vec<String> = packages.iter().map(package_name).collect();
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut violations = Vec::new();

    for package in packages {
        let owner = package_name(package);
        for binary in binary_targets(package) {
            owners.entry(binary).or_default().push(owner.clone());
        }
    }

    for (binary, mut sharing) in owners {
        if sharing.len() < 2 {
            continue;
        }
        sharing.sort();
        let listed = match sharing.split_last() {
            Some((last, [])) => format!("`{last}`"),
            Some((last, rest)) => format!(
                "{} and `{last}`",
                rest.iter()
                    .map(|owner| format!("`{owner}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => String::new(),
        };
        violations.push(format!(
            "{listed} each ship a binary called `{binary}`; two packages offering the same binary name collide in `target/` on every workspace build and make `cargo install` fail outright for anyone who installs both, which is a permanent problem once either name is claimed on a registry"
        ));
    }

    for package in packages {
        let owner = package_name(package);
        for binary in binary_targets(package) {
            if binary != owner && names.contains(&binary) {
                violations.push(format!(
                    "`{owner}` ships a binary called `{binary}`, and `{binary}` is also a package in this workspace; two packages offering the same binary name collide in `target/` on every workspace build and make `cargo install` fail outright for anyone who installs both, which is a permanent problem once either name is claimed on a registry"
                ));
            }
        }
    }

    violations
}

#[test]
fn no_binary_target_takes_a_name_another_package_owns() {
    let violations = binary_name_collisions(&workspace_packages());
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

fn metadata_package(name: &str, binaries: &[&str]) -> serde_json::Value {
    let mut targets = vec![serde_json::json!({"kind": ["lib"], "name": name})];
    for binary in binaries {
        targets.push(serde_json::json!({"kind": ["bin"], "name": binary}));
    }
    serde_json::json!({"name": name, "targets": targets})
}

#[test]
fn two_packages_shipping_one_binary_name_are_reported_even_when_no_package_bears_it() {
    let packages = [
        metadata_package("alpha", &["runner"]),
        metadata_package("beta", &["runner"]),
        metadata_package("gamma", &[]),
    ];

    let violations = binary_name_collisions(&packages);

    assert_eq!(
        violations.len(),
        1,
        "`alpha` and `beta` both ship a binary called `runner` while no package bears that name, so the package-name check alone never sees this collision and nothing else in the workspace would report it; it reported {violations:?}"
    );
}

#[test]
fn a_binary_named_after_its_own_package_is_not_reported() {
    let packages = [
        metadata_package("alpha", &["alpha"]),
        metadata_package("beta", &["beta"]),
    ];

    let violations = binary_name_collisions(&packages);

    assert!(
        violations.is_empty(),
        "a package shipping a binary under its own name takes nothing from anybody, and reporting it would make the guard refuse the ordinary case; it reported {violations:?}"
    );
}

#[test]
fn a_binary_taking_another_packages_name_is_still_reported() {
    let packages = [
        metadata_package("alpha", &["beta"]),
        metadata_package("beta", &[]),
    ];

    let violations = binary_name_collisions(&packages);

    assert_eq!(
        violations.len(),
        1,
        "`alpha` ships a binary called `beta` while `beta` is a package in this workspace, which is the collision the guard already refused before it also learned about duplicate binaries; it reported {violations:?}"
    );
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
