use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use plasmosome_guards::check_workspace;

const TESTKIT: &str = "plasmosome-testkit";

const LANES: &str = "plasmosome-lanes";

const TOOLS: &str = "plasmosome-tools";

const BACKEND: &str = "plasmosome-backend";

const TEST_FIXTURE: &str = "test-fixture";

const HELD_NAMES: &[&str] = &["plasmosome", "plasmid"];

const HELD_REGISTRIES: &[&str] = &["crates-io"];

fn cargo_in(root: &Path) -> Command {
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
    command.current_dir(root);
    command
}

fn workspace_members(root: &Path) -> Vec<String> {
    let members = workspace_members_in(root);
    assert!(
        members.contains(&TESTKIT.to_string()),
        "the workspace manifest no longer lists its members one per line: {members:?}"
    );
    members
}

fn declared_package_name(root: &Path, path: &str) -> String {
    let manifest_path = root.join(path).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "`{path}` is listed as a workspace member and its manifest at {manifest_path:?} could not be read ({error}); this guard resolves every member path to the package that member declares, and a member it cannot read is refused rather than dropped, because dropping it would leave the member count and every name-based check reporting a workspace smaller than the one on disk"
        )
    });
    let document: toml::Value = manifest.parse().unwrap_or_else(|error| {
        panic!(
            "`{path}` is listed as a workspace member and its manifest is not valid TOML ({error}); this guard resolves every member path to the package that member declares, and a member it cannot parse is refused rather than dropped, because dropping it would leave the member count and every name-based check reporting a workspace smaller than the one on disk"
        )
    });
    document
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or_else(|| {
            panic!(
                "`{path}` declares no `[package].name` string (the key is absent, or holds a value that is not a string); this guard resolves every member path to the package that member declares and refuses to continue on a member it cannot take a name from, because dropping it would leave the member count and every name-based check reporting a workspace smaller than the one on disk"
            )
        })
        .to_string()
}

fn workspace_members_in(root: &Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))
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
            members.push(declared_package_name(root, path));
        }
    }
    members
}

fn workspace_packages(root: &Path) -> Vec<serde_json::Value> {
    let output = cargo_in(root)
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
    check_workspace(|root| {
        let mut reported = Vec::new();
        for package in workspace_packages(root) {
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

        let listed = workspace_members(root).len();
        assert_eq!(
            reported.len(),
            listed,
            "the workspace manifest lists {listed} members but `cargo metadata` reported {}, so this rule cannot claim to have checked them all; it checked {reported:?}",
            reported.len()
        );
    });
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
    check_workspace(|root| {
        let violations = binary_name_collisions(&workspace_packages(root));
        assert!(violations.is_empty(), "{}", violations.join("\n"));
    });
}

#[test]
fn the_plasmosome_binary_is_in_the_held_package() {
    check_workspace(|root| {
        let packages = workspace_packages(root);
        let package = |name: &str| {
            packages
                .iter()
                .find(|package| package_name(package) == name)
                .unwrap_or_else(|| panic!("`{name}` is not a package in this workspace"))
        };
        assert_eq!(
            binary_targets(package("plasmosome")),
            ["plasmosome"],
            "spec 025 §1 makes `plasmosome` a binary target of the `plasmosome` package, the name this project holds"
        );
        assert_eq!(
            package("plasmosome-cli")["publish"],
            serde_json::json!([]),
            "spec 025 §1 keeps the command line's logic in `plasmosome-cli` with `publish = false`, so the held `plasmosome` name stays the only publishable package it ships in"
        );
    });
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

fn shipping_graph(root: &Path, member: &str) -> String {
    let output = cargo_in(root)
        .args([
            "tree",
            "--locked",
            "-p",
            member,
            "--edges",
            "normal,build",
            "--all-features",
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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn members_shipping(root: &Path, members: &[String], package: &str) -> Vec<String> {
    let node = format!("{package} ");
    members
        .iter()
        .filter(|member| *member != package)
        .filter(|member| {
            shipping_graph(root, member)
                .lines()
                .any(|line| line.starts_with(&node))
        })
        .cloned()
        .collect()
}

fn testkit_refusals(root: &Path, members: &[String]) -> Vec<String> {
    members_shipping(root, members, TESTKIT)
        .into_iter()
        .map(|member| {
            format!(
                "`{member}` has a non-dev dependency path to `{TESTKIT}`; the testkit is test support and reaches a kernel crate only through `[dev-dependencies]`, or it ships"
            )
        })
        .collect()
}

fn lanes_refusals(root: &Path, members: &[String]) -> Vec<String> {
    members_shipping(root, members, LANES)
        .into_iter()
        .map(|member| {
            format!(
                "`{member}` has a non-dev dependency path to `{LANES}`. The lanes are a test fixture that loads libkrun and boots a guest, so their code would ship inside a product binary (spec 013). Reach them only through `[dev-dependencies]`, never through a normal, build or optional dependency."
            )
        })
        .collect()
}

#[test]
fn testkit_is_dev_only() {
    check_workspace(|root| {
        let refusals = testkit_refusals(root, &workspace_members(root));
        assert!(refusals.is_empty(), "{}", refusals.join("\n"));
    });
}

#[test]
fn lanes_are_dev_only() {
    check_workspace(|root| {
        let members = workspace_members(root);
        assert!(
            members.iter().any(|member| member == LANES),
            "`{LANES}` is no longer a workspace member, so this guard checks nothing: {members:?}"
        );
        let refusals = lanes_refusals(root, &members);
        assert!(refusals.is_empty(), "{}", refusals.join("\n"));
    });
}

fn tools_refusals(root: &Path, members: &[String]) -> Vec<String> {
    if !members.iter().any(|member| member == TOOLS) {
        return vec![format!(
            "`{TOOLS}` is no longer a workspace member, so this guard checks nothing: {members:?}"
        )];
    }
    members_shipping(root, members, TOOLS)
        .into_iter()
        .map(|member| {
            format!(
                "`{member}` has a non-dev dependency path to `{TOOLS}`. The tools are the repository's CI and maintainer tooling, not product code, so their code would ship inside a product binary (spec 013). Reach them only through `[dev-dependencies]`, never through a normal, build or optional dependency."
            )
        })
        .collect()
}

#[test]
fn tools_are_dev_only() {
    check_workspace(|root| {
        let refusals = tools_refusals(root, &workspace_members(root));
        assert!(refusals.is_empty(), "{}", refusals.join("\n"));
    });
}

fn tools_refusals_for(sections: &str) -> Vec<String> {
    let workspace = fixture_feature_workspace_of(
        &[
            (
                "crates/membrane",
                &consumer("plasmosome-membrane", sections),
            ),
            ("crates/tools", &member_manifest(TOOLS)),
        ],
        "",
    );
    tools_refusals(workspace.path(), &workspace_members_in(workspace.path()))
}

fn assert_membrane_ships_the_tools(refusals: &[String]) {
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert!(
        refusals[0].starts_with(
            "`plasmosome-membrane` has a non-dev dependency path to `plasmosome-tools`"
        ),
        "{refusals:?}"
    );
}

#[test]
fn a_normal_dependency_on_the_tools_is_refused() {
    assert_membrane_ships_the_tools(&tools_refusals_for(
        "[dependencies]\nplasmosome-tools = { path = \"../tools\" }\n",
    ));
}

#[test]
fn a_build_dependency_on_the_tools_is_refused() {
    assert_membrane_ships_the_tools(&tools_refusals_for(
        "[build-dependencies]\nplasmosome-tools = { path = \"../tools\" }\n",
    ));
}

/// CI runs this guard on Linux and on macOS, so a dependency only for Windows is never the host's.
#[test]
fn a_target_specific_dependency_on_the_tools_is_refused() {
    assert_membrane_ships_the_tools(&tools_refusals_for(
        "[target.'cfg(windows)'.dependencies]\nplasmosome-tools = { path = \"../tools\" }\n",
    ));
}

#[test]
fn an_optional_tools_dependency_behind_a_feature_is_refused() {
    assert_membrane_ships_the_tools(&tools_refusals_for(
        "[features]\ntools = [\"dep:plasmosome-tools\"]\n\n[dependencies]\nplasmosome-tools = { path = \"../tools\", optional = true }\n",
    ));
}

#[test]
fn a_dev_dependency_on_the_tools_is_not_refused() {
    let refusals =
        tools_refusals_for("[dev-dependencies]\nplasmosome-tools = { path = \"../tools\" }\n");

    assert!(refusals.is_empty(), "{refusals:?}");
}

#[test]
fn a_workspace_without_the_tools_is_refused_not_passed() {
    let workspace = fixture_feature_workspace(&consumer("plasmosome-membrane", ""), "");

    let refusals = tools_refusals(workspace.path(), &workspace_members_in(workspace.path()));

    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert!(
        refusals[0].starts_with("`plasmosome-tools` is no longer a workspace member"),
        "{refusals:?}"
    );
}

fn membrane_workspace(sections: &str) -> tempfile::TempDir {
    fixture_feature_workspace_of(
        &[
            (
                "crates/membrane",
                &consumer("plasmosome-membrane", sections),
            ),
            ("crates/lanes", &member_manifest(LANES)),
            ("crates/testkit", &member_manifest(TESTKIT)),
        ],
        "",
    )
}

fn refusals_for(sections: &str) -> (Vec<String>, Vec<String>) {
    let workspace = membrane_workspace(sections);
    let members = workspace_members_in(workspace.path());
    (
        lanes_refusals(workspace.path(), &members),
        testkit_refusals(workspace.path(), &members),
    )
}

#[test]
fn a_normal_dependency_on_the_lanes_or_the_testkit_is_refused() {
    let (lanes, testkit) = refusals_for(
        "[dependencies]\nplasmosome-lanes = { path = \"../lanes\" }\nplasmosome-testkit = { path = \"../testkit\" }\n",
    );

    assert_eq!(lanes.len(), 1, "{lanes:?}");
    assert!(
        lanes[0].starts_with(
            "`plasmosome-membrane` has a non-dev dependency path to `plasmosome-lanes`"
        ),
        "{lanes:?}"
    );
    assert_eq!(testkit.len(), 1, "{testkit:?}");
    assert!(
        testkit[0].starts_with(
            "`plasmosome-membrane` has a non-dev dependency path to `plasmosome-testkit`"
        ),
        "{testkit:?}"
    );
}

#[test]
fn an_optional_lanes_dependency_behind_a_feature_is_refused() {
    let (lanes, testkit) = refusals_for(
        "[features]\nlanes = [\"dep:plasmosome-lanes\"]\n\n[dependencies]\nplasmosome-lanes = { path = \"../lanes\", optional = true }\n",
    );

    assert_eq!(
        lanes.len(),
        1,
        "a feature that links the lanes into the membrane is missed: {lanes:?}"
    );
    assert!(
        lanes[0].starts_with(
            "`plasmosome-membrane` has a non-dev dependency path to `plasmosome-lanes`"
        ),
        "{lanes:?}"
    );
    assert!(testkit.is_empty(), "{testkit:?}");
}

#[test]
fn an_optional_testkit_dependency_behind_a_feature_is_refused() {
    let (lanes, testkit) = refusals_for(
        "[features]\nfixtures = [\"dep:plasmosome-testkit\"]\n\n[dependencies]\nplasmosome-testkit = { path = \"../testkit\", optional = true }\n",
    );

    assert_eq!(
        testkit.len(),
        1,
        "a feature that links the testkit into the membrane is missed: {testkit:?}"
    );
    assert!(
        testkit[0].starts_with(
            "`plasmosome-membrane` has a non-dev dependency path to `plasmosome-testkit`"
        ),
        "{testkit:?}"
    );
    assert!(lanes.is_empty(), "{lanes:?}");
}

#[test]
fn dev_dependencies_and_a_feature_that_turns_on_no_dependency_are_not_refused() {
    let (lanes, testkit) = refusals_for(
        "[features]\ntest-fixture = []\n\n[dev-dependencies]\nplasmosome-lanes = { path = \"../lanes\" }\nplasmosome-testkit = { path = \"../testkit\" }\n",
    );

    assert!(lanes.is_empty(), "{lanes:?}");
    assert!(testkit.is_empty(), "{testkit:?}");
}

fn write_member(root: &Path, directory: &str, manifest: &str) {
    let member = root.join(directory);
    std::fs::create_dir_all(member.join("src")).expect("the fixture member directory is created");
    std::fs::write(member.join("Cargo.toml"), manifest)
        .expect("the fixture member manifest is written");
    std::fs::write(member.join("src").join("lib.rs"), "")
        .expect("the fixture member source is written");
}

fn write_workspace_manifest(root: &Path, member_paths: &[&str]) {
    let mut manifest = String::from("[workspace]\nresolver = \"2\"\nmembers = [\n");
    for member_path in member_paths {
        manifest.push_str(&format!("    \"{member_path}\",\n"));
    }
    manifest.push_str("]\n");
    std::fs::write(root.join("Cargo.toml"), manifest)
        .expect("the fixture workspace manifest is written");
}

fn member_manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
}

fn fixture_workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let root = directory.path();
    write_workspace_manifest(root, &["crates/plasmid-placeholder", "crates/straight"]);
    write_member(
        root,
        "crates/plasmid-placeholder",
        &member_manifest("plasmid"),
    );
    write_member(root, "crates/straight", &member_manifest("straight"));
    directory
}

fn workspace_with_a_member_declaring_no_package() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let root = directory.path();
    write_workspace_manifest(root, &["crates/straight", "crates/nameless"]);
    write_member(root, "crates/straight", &member_manifest("straight"));
    write_member(root, "crates/nameless", "[lib]\n");
    directory
}

fn workspace_with_a_member_whose_package_name_is_not_a_string() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let root = directory.path();
    write_workspace_manifest(root, &["crates/straight", "crates/numbered"]);
    write_member(root, "crates/straight", &member_manifest("straight"));
    write_member(
        root,
        "crates/numbered",
        "[package]\nname = 123\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    directory
}

fn workspace_with_a_member_whose_manifest_is_missing() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let root = directory.path();
    write_workspace_manifest(root, &["crates/straight", "crates/vanished"]);
    write_member(root, "crates/straight", &member_manifest("straight"));
    std::fs::create_dir_all(root.join("crates").join("vanished"))
        .expect("the fixture member directory is created");
    directory
}

fn workspace_with_a_member_whose_manifest_is_not_valid_toml() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let root = directory.path();
    write_workspace_manifest(root, &["crates/straight", "crates/garbled"]);
    write_member(root, "crates/straight", &member_manifest("straight"));
    write_member(root, "crates/garbled", "[package\nname = \"garbled\"\n");
    directory
}

#[test]
fn a_member_directory_that_differs_from_its_package_name_yields_the_declared_name() {
    let workspace = fixture_workspace();

    let members = workspace_members_in(workspace.path());

    assert_eq!(
        members,
        vec!["plasmid".to_string(), "straight".to_string()],
        "`crates/plasmid-placeholder` declares the package `plasmid` and `crates/straight` declares `straight`; a member list that reports a directory in place of the package that directory declares hands every guard built on it a name Cargo cannot resolve, and the guard then fails on the crate that provoked it instead of on the rule it enforces; it reported {members:?}"
    );
}

#[test]
fn cargo_tree_resolves_every_name_workspace_members_reports() {
    let workspace = fixture_workspace();
    let names = workspace_members_in(workspace.path());

    assert_eq!(
        names,
        vec!["plasmid".to_string(), "straight".to_string()],
        "the names checked below are the ones this list holds, so an empty or shortened list would let the loop pass having resolved nothing; it reported {names:?}"
    );

    for name in names {
        let output = cargo_in(workspace.path())
            .args(["tree", "-p", &name, "--prefix", "none"])
            .output()
            .expect("cargo tree runs");
        assert!(
            output.status.success(),
            "cargo tree -p {name} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let graph = String::from_utf8_lossy(&output.stdout);
        let root_of_the_graph = graph.lines().next().unwrap_or_default();
        assert!(
            root_of_the_graph.starts_with(&format!("{name} v0.1.0")),
            "`cargo tree -p {name}` succeeded but rooted its graph at {root_of_the_graph:?}; the name this guard reports has to be the package Cargo resolves it to, or a command that merely exits zero would stand in for one that found the crate"
        );
    }
}

#[test]
fn a_member_that_declares_no_package_name_is_refused_not_skipped() {
    let workspace = workspace_with_a_member_declaring_no_package();

    let result = std::panic::catch_unwind(|| workspace_members_in(workspace.path()));

    assert!(result.is_err(), "a nameless member must not be skipped");
}

#[test]
fn a_member_whose_package_name_is_not_a_string_is_refused_not_skipped() {
    let workspace = workspace_with_a_member_whose_package_name_is_not_a_string();

    let result = std::panic::catch_unwind(|| workspace_members_in(workspace.path()));

    assert!(
        result.is_err(),
        "a non-string package name must not be coerced or skipped"
    );
}

#[test]
fn a_member_whose_manifest_is_missing_is_refused_not_skipped() {
    let workspace = workspace_with_a_member_whose_manifest_is_missing();

    let result = std::panic::catch_unwind(|| workspace_members_in(workspace.path()));

    assert!(result.is_err(), "an unreadable member must not be skipped");
}

#[test]
fn a_member_whose_manifest_is_not_valid_toml_is_refused_not_skipped() {
    let workspace = workspace_with_a_member_whose_manifest_is_not_valid_toml();

    let result = std::panic::catch_unwind(|| workspace_members_in(workspace.path()));

    assert!(
        result.is_err(),
        "an invalid member manifest must not be skipped"
    );
}

#[test]
fn no_crate_enables_the_backend_test_fixture() {
    check_workspace(|root| {
        if let Some(refusal) = test_fixture_refusal(root) {
            panic!("{refusal}");
        }
    });
}

fn test_fixture_refusal(root: &Path) -> Option<String> {
    let enablers = test_fixture_enablers(root);
    (!enablers.is_empty()).then(|| {
        format!(
            "{enablers:?} enable `{BACKEND}/{TEST_FIXTURE}` outside `[dev-dependencies]`. That feature lets code build a `CommandName` from text, so test scaffolding would ship to a consumer, which spec 013 puts in a guard's scope as \"test scaffolding leaving `[dev-dependencies]`\". No crate may enable it, `{TESTKIT}` included: a `--workspace` build merges the features of every member it selects, so one crate's edge turns the feature on for all of them, and every gate here builds that way. A crate that needs the fixture must, in the same PR, add a `cargo check -p` for each product crate and change this guard"
        )
    })
}

fn test_fixture_enablers(root: &Path) -> Vec<String> {
    let output = cargo_in(root)
        .args([
            "tree",
            "--locked",
            "--workspace",
            "--edges",
            "normal,build,features",
            "--invert",
            BACKEND,
            "--target",
            "all",
            "--prefix",
            "depth",
        ])
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree --invert {BACKEND} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let graph = String::from_utf8_lossy(&output.stdout);
    let nodes: Vec<(usize, &str)> = graph
        .lines()
        .filter_map(|line| {
            let text = line.trim_start_matches(|character: char| character.is_ascii_digit());
            let depth = line[..line.len() - text.len()].parse().ok()?;
            Some((depth, text))
        })
        .collect();
    assert!(
        nodes
            .first()
            .is_some_and(|(depth, text)| *depth == 0 && text.starts_with(&format!("{BACKEND} v"))),
        "the inverted graph is not rooted at `{BACKEND}`, so this guard would read nothing: {graph}"
    );
    let feature = format!("{BACKEND} feature \"{TEST_FIXTURE}\"");
    let mut enablers = std::collections::BTreeSet::new();
    for (at, (depth, text)) in nodes.iter().enumerate() {
        if text.starts_with(&feature) {
            enablers.extend(
                nodes[at + 1..]
                    .iter()
                    .take_while(|(below, _)| below > depth)
                    .filter(|(below, _)| *below == depth + 1)
                    .map(|(_, text)| text.split(' ').next().unwrap_or_default().to_string()),
            );
        }
    }
    enablers.into_iter().collect()
}

fn fixture_feature_workspace(consumer: &str, workspace_dependencies: &str) -> tempfile::TempDir {
    fixture_feature_workspace_of(&[("crates/consumer", consumer)], workspace_dependencies)
}

fn fixture_feature_workspace_of(
    consumers: &[(&str, &str)],
    workspace_dependencies: &str,
) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let root = directory.path();
    let mut members = vec!["crates/backend"];
    members.extend(consumers.iter().map(|(path, _)| *path));
    write_workspace_manifest(root, &members);
    if !workspace_dependencies.is_empty() {
        let mut manifest =
            std::fs::read_to_string(root.join("Cargo.toml")).expect("the manifest reads back");
        manifest.push_str(workspace_dependencies);
        std::fs::write(root.join("Cargo.toml"), manifest).expect("the manifest is rewritten");
    }
    write_member(
        root,
        "crates/backend",
        &format!(
            "{}\n[features]\n{TEST_FIXTURE} = []\n",
            member_manifest(BACKEND)
        ),
    );
    for (path, manifest) in consumers {
        write_member(root, path, manifest);
    }
    let locked = cargo_in(root)
        .args(["generate-lockfile", "--offline"])
        .output()
        .expect("cargo generate-lockfile runs");
    assert!(
        locked.status.success(),
        "the fixture workspace cannot be locked: {}",
        String::from_utf8_lossy(&locked.stderr)
    );
    directory
}

fn consumer(name: &str, sections: &str) -> String {
    format!("{}\n{sections}", member_manifest(name))
}

#[test]
fn a_member_that_forwards_the_test_fixture_through_its_own_features_is_reported() {
    let workspace = fixture_feature_workspace(
        &consumer(
            "plasmosome-membrane",
            "[features]\ndefault = [\"fixtures\"]\nfixtures = [\"plasmosome-backend/test-fixture\"]\n\n[dependencies]\nplasmosome-backend = { path = \"../backend\" }\n",
        ),
        "",
    );
    assert_eq!(
        test_fixture_enablers(workspace.path()),
        ["plasmosome-membrane"]
    );
}

#[test]
fn a_crate_that_only_depends_on_the_enabler_is_not_named() {
    let enabler = consumer(
        "plasmosome-membrane",
        "[dependencies]\nplasmosome-backend = { path = \"../backend\", default-features = false, features = [\"test-fixture\"] }\n",
    );
    let dependent = consumer(
        "plasmosome-controller",
        "[dependencies]\nplasmosome-membrane = { path = \"../membrane\" }\n",
    );
    let workspace = fixture_feature_workspace_of(
        &[
            ("crates/membrane", &enabler),
            ("crates/controller", &dependent),
        ],
        "",
    );
    assert_eq!(
        test_fixture_enablers(workspace.path()),
        ["plasmosome-membrane"]
    );
}

#[test]
fn a_workspace_dependency_that_enables_the_test_fixture_is_reported() {
    let workspace = fixture_feature_workspace(
        &consumer(
            "plasmosome-core",
            "[dependencies]\nplasmosome-backend = { workspace = true }\n",
        ),
        "\n[workspace.dependencies]\nplasmosome-backend = { path = \"crates/backend\", features = [\"test-fixture\"] }\n",
    );
    assert_eq!(test_fixture_enablers(workspace.path()), ["plasmosome-core"]);
}

#[test]
fn a_dependency_for_one_target_that_enables_the_test_fixture_is_reported() {
    for target in [
        "cfg(target_os = \"linux\")",
        "cfg(target_os = \"macos\")",
        "cfg(windows)",
    ] {
        let workspace = fixture_feature_workspace(
            &consumer(
                "plasmosome-ledger",
                &format!(
                    "[target.'{target}'.dependencies]\nplasmosome-backend = {{ path = \"../backend\", features = [\"test-fixture\"] }}\n"
                ),
            ),
            "",
        );
        assert_eq!(
            test_fixture_enablers(workspace.path()),
            ["plasmosome-ledger"],
            "a dependency for {target} is missed"
        );
    }
}

#[test]
fn a_dev_dependency_that_enables_the_test_fixture_is_not_reported() {
    let workspace = fixture_feature_workspace(
        &consumer(
            "plasmosome-core",
            "[dependencies]\nplasmosome-backend = { path = \"../backend\" }\n\n[dev-dependencies]\nplasmosome-backend = { path = \"../backend\", features = [\"test-fixture\"] }\n",
        ),
        "",
    );
    assert_eq!(
        test_fixture_enablers(workspace.path()),
        Vec::<String>::new()
    );
}

#[test]
fn a_build_dependency_that_enables_the_test_fixture_is_reported() {
    let workspace = fixture_feature_workspace(
        &consumer(
            "plasmosome-ledger",
            "[build-dependencies]\nplasmosome-backend = { path = \"../backend\", features = [\"test-fixture\"] }\n",
        ),
        "",
    );
    assert_eq!(
        test_fixture_enablers(workspace.path()),
        ["plasmosome-ledger"]
    );
}

#[test]
fn the_testkit_enabling_the_test_fixture_is_refused_like_any_crate() {
    let workspace = fixture_feature_workspace(
        &consumer(
            TESTKIT,
            "[dependencies]\nplasmosome-backend = { path = \"../backend\", features = [\"test-fixture\"] }\n",
        ),
        "",
    );
    let refusal = test_fixture_refusal(workspace.path())
        .expect("a testkit edge turns the feature on for every member of a workspace build");
    assert!(refusal.contains(&format!("[\"{TESTKIT}\"]")), "{refusal}");
    assert!(refusal.contains("`CommandName` from text"), "{refusal}");
    assert!(refusal.contains("`cargo check -p`"), "{refusal}");
}

#[test]
fn a_workspace_where_no_crate_enables_the_test_fixture_is_not_refused() {
    let workspace = fixture_feature_workspace(
        &consumer(
            "plasmosome-core",
            "[dependencies]\nplasmosome-backend = { path = \"../backend\" }\n",
        ),
        "",
    );
    assert_eq!(test_fixture_refusal(workspace.path()), None);
}

fn matrix_read_error(line: &str, why: &str) -> String {
    format!(
        "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as \
         `        crate:` followed by `          - <package>` lines, blank lines and comments \
         allowed, and it met `{line}` ({why}). A reader that guessed could report a list it \
         did not read. Write the list in that form, or teach this guard the new form in the \
         same PR, with a test for it."
    )
}

fn workflow_block_scalar_indent(line: &str) -> Option<usize> {
    let text = line.trim();
    let indent = line.len() - line.trim_start_matches(' ').len();
    let header = text.split_once(" #").map_or(text, |(header, _)| header);
    let field = header.split_once(':');
    let value = field
        .map(|(_, value)| value.trim_start())
        .unwrap_or_else(|| header.strip_prefix("- ").unwrap_or(header));
    let indicator = value
        .split_whitespace()
        .find(|word| !word.starts_with(['&', '!']))
        .unwrap_or("");
    (indicator.starts_with(['|', '>'])
        && indicator[1..]
            .bytes()
            .all(|byte| matches!(byte, b'+' | b'-' | b'1'..=b'9')))
    .then_some(indent + usize::from(text.starts_with("- ") && field.is_some()) * 2)
}

fn matrix_structure_lines(workflow: &str) -> Vec<&str> {
    let mut scalar_indent = None;
    let mut lines = Vec::new();
    for line in workflow.lines() {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if let Some(parent_indent) = scalar_indent {
            if indent > parent_indent {
                continue;
            }
            scalar_indent = None;
        }
        if text.starts_with('#') {
            continue;
        }
        scalar_indent = workflow_block_scalar_indent(line);
        lines.push(line);
    }
    lines
}

fn workflow_line_error(number: usize, line: &str, cause: &str) -> String {
    format!(
        "workflow line {number} is refused ({cause}): `{line}`. The matrix guard accepts only \
         blank lines, comments, plain-key mappings and sequence entries with values complete \
         on one line, and block scalars. A reader that guessed could report a matrix it did not read."
    )
}

fn workflow_value_end(value: &str) -> Result<(), &'static str> {
    let mut chars = value.char_indices().peekable();
    let Some((_, first)) = chars.next() else {
        return Ok(());
    };
    let mut quote = match first {
        '\'' | '"' => Some(first),
        _ => None,
    };
    let mut closers = match first {
        '[' => vec![']'],
        '{' => vec!['}'],
        _ if quote.is_none() => return Ok(()),
        _ => Vec::new(),
    };
    let quoted = quote.is_some();
    while let Some((index, character)) = chars.next() {
        if let Some(delimiter) = quote {
            if delimiter == '"' && character == '\\' {
                chars.next();
                continue;
            }
            if character != delimiter {
                continue;
            }
            if delimiter == '\'' && chars.peek().is_some_and(|(_, next)| *next == '\'') {
                chars.next();
                continue;
            }
            quote = None;
        } else {
            match character {
                '\'' | '"' => quote = Some(character),
                '[' => closers.push(']'),
                '{' => closers.push('}'),
                ']' | '}' if closers.pop() != Some(character) => {
                    return Err("mismatched flow collection");
                }
                '#' if value[..index].ends_with(char::is_whitespace) => break,
                _ => {}
            }
        }
        if quote.is_none() && closers.is_empty() {
            let rest = &value[index + character.len_utf8()..];
            return if rest.trim().is_empty()
                || (rest.starts_with(char::is_whitespace) && rest.trim_start().starts_with('#'))
            {
                Ok(())
            } else {
                Err("text after a complete value")
            };
        }
    }
    Err(if quoted {
        "quoted scalar does not close on its line"
    } else {
        "flow collection does not close on its line"
    })
}

fn workflow_line_grammar(workflow: &str) -> Result<(), String> {
    let mut scalar_parent = None;
    let mut block_parent = None;
    for (index, line) in workflow.lines().enumerate() {
        let number = index + 1;
        let indent = line.len() - line.trim_start_matches(' ').len();
        let text = &line[indent..];
        if line.is_empty() {
            continue;
        }
        if let Some(parent) = block_parent {
            if indent > parent || text.is_empty() {
                continue;
            }
            block_parent = None;
        }
        let refuse = |cause| workflow_line_error(number, line, cause);
        if text.starts_with('\t') {
            return Err(refuse("tab in indentation"));
        }
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        if scalar_parent.is_some_and(|parent| indent > parent) {
            return Err(refuse("plain-scalar continuation"));
        }
        let (content, key_indent, sequence) = match text.strip_prefix("- ") {
            Some(content) => (content, indent + 2, true),
            None => (text, indent, false),
        };
        let field = content
            .split_once(':')
            .filter(|(_, value)| value.is_empty() || value.starts_with(char::is_whitespace))
            .filter(|_| {
                !(sequence
                    && content.starts_with(['\'', '"'])
                    && workflow_value_end(content).is_ok())
            });
        let value = if let Some((key, value)) = field {
            if key.starts_with(['\'', '"']) {
                return Err(refuse("quoted key"));
            }
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
            {
                return Err(refuse("unrecognised line form"));
            }
            value.trim_start()
        } else if sequence {
            content
        } else {
            return Err(refuse("unrecognised line form"));
        };
        workflow_value_end(value).map_err(refuse)?;
        block_parent = workflow_block_scalar_indent(line);
        if value.starts_with(['&', '!'])
            && value.split_whitespace().count() > 1
            && block_parent.is_none()
        {
            return Err(refuse("unrecognised line form"));
        }
        scalar_parent = if value.is_empty() || value.starts_with('#') || block_parent.is_some() {
            None
        } else {
            Some(if field.is_some() { key_indent } else { indent })
        };
    }
    Ok(())
}

fn matrix_block<'a>(
    lines: &[&'a str],
    indent: usize,
    key: &str,
) -> Result<Option<Vec<&'a str>>, String> {
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.len() - line.trim_start_matches(' ').len() == indent
                && line.trim_start().split_once(':').map(|(name, _)| name) == Some(key)
        })
        .map(|(index, _)| index)
        .collect();
    let start = match starts.as_slice() {
        [] => return Ok(None),
        [start] => *start,
        _ => {
            return Err(matrix_read_error(
                lines[starts[1]],
                &format!("more than one {key} mapping"),
            ));
        }
    };
    let value = lines[start].split_once(':').unwrap().1.trim();
    if !value.is_empty() && !value.starts_with('#') {
        return Err(matrix_read_error(
            lines[start],
            &format!("{key} must be a block mapping"),
        ));
    }
    Ok(Some(
        lines[start + 1..]
            .iter()
            .copied()
            .take_while(|line| line.len() - line.trim_start_matches(' ').len() > indent)
            .collect(),
    ))
}

fn crate_matrix_entries(workflow: &str) -> Result<Vec<String>, String> {
    workflow_line_grammar(workflow)?;
    let lines = matrix_structure_lines(workflow);
    let Some(jobs) = matrix_block(&lines, 0, "jobs")? else {
        return Ok(Vec::new());
    };
    let Some(job) = matrix_block(&jobs, 2, "crate")? else {
        return Ok(Vec::new());
    };
    let strategy = matrix_block(&job, 4, "strategy")?
        .ok_or_else(|| matrix_read_error("  crate:", "no block strategy"))?;
    let matrix = matrix_block(&strategy, 6, "matrix")?
        .ok_or_else(|| matrix_read_error("    strategy:", "no block matrix"))?;
    let mut matrix_lines = matrix.iter().copied();
    match matrix_lines.next() {
        Some("        crate:") => {}
        line => {
            return Err(matrix_read_error(
                line.unwrap_or("<end of matrix>"),
                "expected the crate axis before its entries",
            ));
        }
    }
    let mut entries = Vec::new();
    for line in matrix_lines {
        if let Some(name) = line.strip_prefix("          - ") {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(matrix_read_error(line, "expected a bare package name"));
            }
            entries.push(name.to_string());
        } else {
            return Err(matrix_read_error(line, "unsupported matrix content"));
        }
    }
    Ok(entries)
}

fn matrix_drift(entries: &[String], members: &[String]) -> (Vec<String>, Vec<String>) {
    let entries: BTreeSet<_> = entries.iter().cloned().collect();
    let members: BTreeSet<_> = members.iter().cloned().collect();
    (
        members.difference(&entries).cloned().collect(),
        entries.difference(&members).cloned().collect(),
    )
}

fn matrix_fixture(entries: &str) -> String {
    format!(
        "jobs:\n  gates:\n    steps:\n      - uses: checkout\n  crate:\n    strategy:\n      matrix:\n        crate:\n{entries}\n    steps:\n      - uses: checkout\n  msrv:\n    steps:\n      - run: check\n"
    )
}

fn matrix_names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn matrix_grammar_accepts_the_real_workflow() {
    check_workspace(|root| {
        let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
        assert_eq!(workflow_line_grammar(&workflow), Ok(()));
    });
}

fn real_matrix_workflow() -> String {
    let mut workflow = String::new();
    check_workspace(|root| {
        workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    });
    workflow
}

fn full_workflow_name(name: &str) -> String {
    real_matrix_workflow().replacen("    name: crate (${{ matrix.crate }})", name, 1)
}

fn multiline_flow_workflow() -> String {
    let workflow = real_matrix_workflow();
    let strategy = workflow
        .split_once("    strategy:\n")
        .unwrap()
        .1
        .split_once("    steps:\n")
        .unwrap()
        .0;
    workflow.replacen(
        strategy,
        "      matrix:\n        include: [\n          {crate: plasmosome}\n        ]\n",
        1,
    )
}

fn quoted_name_workflow(quote: char) -> String {
    let workflow = real_matrix_workflow();
    let matrix = workflow
        .split_once("    strategy:\n")
        .unwrap()
        .1
        .split_once("    steps:\n")
        .unwrap()
        .0;
    let real_strategy = format!("    strategy:\n{matrix}");
    let workflow = workflow.replacen(
        &real_strategy,
        "    \"strategy\":\n      fail-fast: false\n      matrix:\n        crate:\n          - plasmosome\n",
        1,
    );
    let display_matrix = matrix.replace("      fail-fast: false\n", "");
    workflow.replacen(
        "    name: crate (${{ matrix.crate }})",
        &format!("    name: {quote}display\n    strategy:\n{display_matrix}    {quote}"),
        1,
    )
}

#[test]
fn matrix_grammar_refuses_the_single_quoted_workflow_witness() {
    let workflow = quoted_name_workflow('\'');
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 360 is refused (quoted scalar does not close on its line): `    name: 'display`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_refuses_the_double_quoted_workflow_witness() {
    let workflow = quoted_name_workflow('"');
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 360 is refused (quoted scalar does not close on its line): `    name: \"display`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_refuses_a_quoted_structural_key() {
    let workflow = real_matrix_workflow().replacen("    strategy:", "    \"strategy\":", 1);
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 363 is refused (quoted key): `    \"strategy\":`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_refuses_a_multiline_flow_sequence() {
    let workflow = multiline_flow_workflow();
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 365 is refused (flow collection does not close on its line): `        include: [`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_refuses_a_plain_scalar_continuation() {
    let workflow = full_workflow_name("    name: display\n      continued");
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 361 is refused (plain-scalar continuation): `      continued`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_refuses_tab_indentation() {
    let workflow = full_workflow_name("    \tname: display");
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 360 is refused (tab in indentation): `    \tname: display`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_refuses_an_unrecognised_line() {
    let workflow = full_workflow_name("    ? name");
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 360 is refused (unrecognised line form): `    ? name`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_refuses_scalar_properties_before_a_quoted_value() {
    let workflow = full_workflow_name("    name: &display 'open");
    assert_eq!(crate_matrix_entries(&workflow), Err("workflow line 360 is refused (unrecognised line form): `    name: &display 'open`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.".to_string()));
}

#[test]
fn matrix_grammar_pins_remaining_line_refusals() {
    for (workflow, expected) in [
        (
            "\t# comment\n",
            "workflow line 1 is refused (tab in indentation): `\t# comment`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.",
        ),
        (
            ": value\n",
            "workflow line 1 is refused (unrecognised line form): `: value`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.",
        ),
        (
            "bad key: value\n",
            "workflow line 1 is refused (unrecognised line form): `bad key: value`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.",
        ),
        (
            "key: [}\n",
            "workflow line 1 is refused (mismatched flow collection): `key: [}`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.",
        ),
        (
            "key: 'closed' more\n",
            "workflow line 1 is refused (text after a complete value): `key: 'closed' more`. The matrix guard accepts only blank lines, comments, plain-key mappings and sequence entries with values complete on one line, and block scalars. A reader that guessed could report a matrix it did not read.",
        ),
    ] {
        assert_eq!(workflow_line_grammar(workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_grammar_values_must_end_on_their_line() {
    for value in [
        "",
        "null",
        "cargo test -p \"$CRATE\"",
        "'it''s complete'",
        "\"escaped \\\" quote\" # note",
        "[]",
        "{}",
        "[main, {key: ['value', \"other\"]}] # note",
        "'complete'  ",
        "\"\"",
        "'single \\ slash'",
    ] {
        assert_eq!(workflow_value_end(value), Ok(()), "{value}");
    }
    for (value, expected) in [
        ("'open", "quoted scalar does not close on its line"),
        ("\"open\\", "quoted scalar does not close on its line"),
        ("[main,", "flow collection does not close on its line"),
        ("{key: value,", "flow collection does not close on its line"),
        ("[main # ]", "flow collection does not close on its line"),
        ("['open]", "flow collection does not close on its line"),
        ("[}", "mismatched flow collection"),
        ("'closed' more", "text after a complete value"),
        ("{}more", "text after a complete value"),
        ("\"closed\"#comment", "text after a complete value"),
    ] {
        assert_eq!(workflow_value_end(value), Err(expected), "{value}");
    }
}

#[test]
fn matrix_grammar_accepts_complete_values_and_block_scalar_bodies() {
    for display in [
        "    name: 'display' # complete",
        "    name: \"display \\\"quoted\\\"\"",
        "    name: [display, {key: 'value'}]",
        "    name: |\n      \tfree form\n      \t# scalar text\n      \"quoted\": [\n      ? anything",
    ] {
        assert_eq!(workflow_line_grammar(&full_workflow_name(display)), Ok(()));
    }
    assert_eq!(
        workflow_line_grammar(
            "\n # comment\nplain_key.0-: # comment\n  - key: value\n    next: other\n  - 'complete: value'\n"
        ),
        Ok(())
    );
}

#[test]
fn matrix_reader_does_not_count_step_entries() {
    let entries = crate_matrix_entries(&matrix_fixture("          - a\n          - b"))
        .expect("a block matrix is readable");
    assert_eq!(entries, matrix_names(&["a", "b"]));
    assert_eq!(matrix_drift(&entries, &entries), (vec![], vec![]));
}

#[test]
fn matrix_comparison_reports_missing_members_in_sorted_order() {
    assert_eq!(
        matrix_drift(&matrix_names(&["a"]), &matrix_names(&["c", "a", "b"])),
        (matrix_names(&["b", "c"]), vec![])
    );
}

#[test]
fn matrix_comparison_reports_non_members_too() {
    assert_eq!(
        matrix_drift(&matrix_names(&["z", "a", "y"]), &matrix_names(&["a"])),
        (vec![], matrix_names(&["y", "z"]))
    );
}

#[test]
fn matrix_reader_skips_comments_without_ending_the_list() {
    let entries = crate_matrix_entries(&matrix_fixture(
        "          - a\n          # - b\n          - c",
    ))
    .expect("comments are allowed");
    assert_eq!(
        matrix_drift(&entries, &matrix_names(&["a", "b", "c"])),
        (matrix_names(&["b"]), vec![])
    );
}

#[test]
fn matrix_reader_skips_blank_lines_without_ending_the_list() {
    assert_eq!(
        crate_matrix_entries(&matrix_fixture("          - a\n\n          - b")),
        Ok(matrix_names(&["a", "b"]))
    );
}

#[test]
fn matrix_reader_does_not_read_another_jobs_matrix() {
    let workflow = format!(
        "{}  other:\n    strategy:\n      matrix:\n        crate:\n          - x\n",
        matrix_fixture("          - a")
    );
    let entries = crate_matrix_entries(&workflow).expect("the crate job is readable");
    assert_eq!(
        matrix_drift(&entries, &matrix_names(&["a", "x"])),
        (matrix_names(&["x"]), vec![])
    );
}

#[test]
fn matrix_reader_refuses_include_and_exclude() {
    for (workflow, expected) in [
        (
            matrix_fixture("          - a\n        include:\n          - crate: a"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `        include:` (unsupported matrix content). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a\n        exclude:\n          - crate: a"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `        exclude:` (unsupported matrix content). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_refuses_flow_lists_expressions_and_other_axes() {
    for (workflow, expected) in [
        (
            matrix_fixture("          - a").replace("crate:\n          - a", "crate: [a, b]"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `        crate: [a, b]` (expected the crate axis before its entries). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "crate:\n          - a",
                "crate: ${{ fromJSON(inputs.crates) }}",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `        crate: ${{ fromJSON(inputs.crates) }}` (expected the crate axis before its entries). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace("crate:\n          - a", "os:"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `        os:` (expected the crate axis before its entries). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_refuses_quoted_empty_and_annotated_names() {
    for (workflow, expected) in [
        (
            matrix_fixture("          - \"a\""),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `          - \"a\"` (expected a bare package name). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a # note"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `          - a # note` (expected a bare package name). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - "),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `          - ` (expected a bare package name). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a/b"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `          - a/b` (expected a bare package name). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - é"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `          - é` (expected a bare package name). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a "),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `          - a ` (expected a bare package name). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_reports_all_members_when_the_job_is_absent() {
    let entries = crate_matrix_entries("jobs:\n  gates:\n    steps: []\n")
        .expect("a missing job has no entries");
    assert_eq!(
        matrix_drift(&entries, &matrix_names(&["b", "a"])),
        (matrix_names(&["a", "b"]), vec![])
    );
}

#[test]
fn matrix_reader_requires_the_jobs_mapping() {
    for workflow in [
        "name: no jobs\n",
        "name: |\n  crate:\n    strategy:\n      matrix:\n        crate:\n          - x\n",
    ] {
        assert_eq!(crate_matrix_entries(workflow), Ok(vec![]));
    }
}

#[test]
fn matrix_reader_refuses_non_block_jobs_and_job() {
    for (workflow, expected) in [
        (
            "jobs: {}\n",
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `jobs: {}` (jobs must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            "jobs:\n  crate: *alias\n",
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `  crate: *alias` (crate must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_refuses_a_job_without_a_strategy_or_matrix() {
    for (workflow, expected) in [
        (
            "jobs:\n  crate:\n    steps: []\n".to_string(),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `  crate:` (no block strategy). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            "jobs:\n  crate:\n    strategy:\n      fail-fast: false\n".to_string(),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy:` (no block matrix). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_refuses_duplicate_mappings() {
    for (workflow, expected) in [
        (
            format!("{}jobs:\n", matrix_fixture("          - a")),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `jobs:` (more than one jobs mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            format!("{}  crate:\n", matrix_fixture("          - a")),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `  crate:` (more than one crate mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a")
                .replace("    strategy:", "    strategy:\n    strategy:"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy:` (more than one strategy mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a")
                .replace("      matrix:", "      matrix:\n      matrix:"),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix:` (more than one matrix mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_accepts_names_comments_and_an_eof_list() {
    let workflow = "jobs:\n  crate:\n    strategy:\n      matrix:\n        # axis\n\n        crate:\n          - A_0-b\n# end\n";
    assert_eq!(crate_matrix_entries(workflow), Ok(matrix_names(&["A_0-b"])));
}

#[test]
fn matrix_reader_refuses_an_empty_matrix() {
    let (workflow, expected) = (
        "jobs:\n  crate:\n    strategy:\n      matrix:\n".to_string(),
        "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `<end of matrix>` (expected the crate axis before its entries). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
    );
    assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
}

#[test]
fn matrix_reader_refuses_non_block_strategy() {
    for (workflow, expected) in [
        (
            matrix_fixture("          - a").replace(
                "    strategy:\n      matrix:\n        crate:\n          - a",
                "    strategy: {}",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy: {}` (strategy must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "    strategy:\n      matrix:\n        crate:\n          - a",
                "    strategy: &anchor",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy: &anchor` (strategy must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "    strategy:\n      matrix:\n        crate:\n          - a",
                "    strategy: *alias",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy: *alias` (strategy must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "    strategy:\n      matrix:\n        crate:\n          - a",
                "    strategy: !!map",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy: !!map` (strategy must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "    strategy:\n      matrix:\n        crate:\n          - a",
                "    strategy: |",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy: |` (strategy must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "    strategy:\n      matrix:\n        crate:\n          - a",
                "    strategy: >-",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `    strategy: >-` (strategy must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_refuses_non_block_matrix() {
    for (workflow, expected) in [
        (
            matrix_fixture("          - a").replace(
                "      matrix:\n        crate:\n          - a",
                "      matrix: {}",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: {}` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "      matrix:\n        crate:\n          - a",
                "      matrix: &anchor",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: &anchor` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "      matrix:\n        crate:\n          - a",
                "      matrix: *alias",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: *alias` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "      matrix:\n        crate:\n          - a",
                "      matrix: !!map",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: !!map` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "      matrix:\n        crate:\n          - a",
                "      matrix: |",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: |` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
        (
            matrix_fixture("          - a").replace(
                "      matrix:\n        crate:\n          - a",
                "      matrix: >-",
            ),
            "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: >-` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.",
        ),
    ] {
        assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
    }
}

#[test]
fn matrix_reader_does_not_read_an_earlier_jobs_matrix() {
    let workflow = matrix_fixture("          - a").replacen(
        "jobs:\n",
        "jobs:\n  other:\n    strategy:\n      matrix:\n        crate:\n          - x\n",
        1,
    );
    assert_eq!(crate_matrix_entries(&workflow), Ok(matrix_names(&["a"])));
}

#[test]
fn matrix_reader_skips_block_scalars_at_every_mapping_level() {
    for indicator in [
        "|", "|-", "|+", "|2", "|2-", "|-2", ">", ">-", ">+", ">2", ">2+", ">+2",
    ] {
        for (before, scalar) in [
            (
                "jobs:\n",
                "name: INDICATOR\n  crate:\n    strategy:\n      matrix:\n        crate:\n          - x\n",
            ),
            (
                "  crate:\n",
                "  other: INDICATOR\n    strategy:\n      matrix:\n        crate:\n          - x\n",
            ),
            (
                "    strategy:\n",
                "    name: INDICATOR\n      matrix:\n        crate:\n          - x\n",
            ),
            (
                "      matrix:\n",
                "      display: INDICATOR\n        crate:\n          - x\n",
            ),
        ] {
            let workflow = matrix_fixture("          - a").replacen(
                before,
                &format!("{}{before}", scalar.replace("INDICATOR", indicator)),
                1,
            );
            assert_eq!(
                crate_matrix_entries(&workflow),
                Ok(matrix_names(&["a"])),
                "{workflow}"
            );
        }
    }
}

#[test]
fn matrix_structure_skips_sequence_scalars_and_keeps_the_next_key() {
    let workflow = "steps:\n  - name: |2-\n      matrix:\n        crate:\n          - x\n    run: echo ok\n  - >+ # display: value\n    strategy:\n      matrix:\nnext: value\n";
    assert_eq!(
        matrix_structure_lines(workflow),
        [
            "steps:",
            "  - name: |2-",
            "    run: echo ok",
            "  - >+ # display: value",
            "next: value"
        ]
    );
}

#[test]
fn matrix_structure_skips_scalars_with_properties_and_comments() {
    for properties in ["", "&label ", "!!str ", "&label !!str ", "!!str &label "] {
        for indicator in ["|", "|-", "|2+", ">", ">+", ">-2"] {
            let header = format!("name: {properties}{indicator} # display");
            let workflow = format!(
                "{header}\n\n  # scalar content\n  jobs:\n    crate:\n      strategy:\n        matrix:\nnext: value\n"
            );
            assert_eq!(
                matrix_structure_lines(&workflow),
                [header, "next: value".to_string()]
            );
        }
    }
}

#[test]
fn matrix_reader_refuses_the_review_fragment() {
    let (workflow, expected) =
        ("jobs:\n  crate:\n    name: |\n      matrix:\n        crate:\n          - a\n          - b\n    strategy:\n      matrix: { crate: [a] }\n".to_string(), "this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: { crate: [a] }` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.")
    ;
    assert_eq!(crate_matrix_entries(&workflow), Err(expected.to_string()));
}

const BLOCK_SCALAR_WORKFLOW: &str = r#"name: matrix scalar regression
on: push
jobs:
  crate:
    name: |
      matrix:
        crate:
          - plasmosome
          - plasmosome-backend
          - plasmosome-ledger
          - plasmosome-core
          - plasmosome-membrane
          - plasmid
          - plasmid-sdk
          - plasmosome-guards
          - plasmosome-testkit
          - plasmosome-cli
          - plasmosome-lanes
          - plasmosome-tools
    runs-on: ubuntu-latest
    strategy:
      matrix: {crate: [plasmosome]}
    steps:
      - uses: actions/checkout@v4
      - name: build
        run: cargo build --locked -p "$CRATE" --all-targets
        env:
          CRATE: ${{ matrix.crate }}
      - name: tests
        run: cargo test --locked -p "$CRATE"
        env:
          CRATE: ${{ matrix.crate }}
"#;

#[test]
fn matrix_reader_refuses_the_full_workflow_scalar_regression() {
    assert_eq!(crate_matrix_entries(BLOCK_SCALAR_WORKFLOW), Err("this guard reads the `crate` job's matrix in .github/workflows/ci.yml only as `        crate:` followed by `          - <package>` lines, blank lines and comments allowed, and it met `      matrix: {crate: [plasmosome]}` (matrix must be a block mapping). A reader that guessed could report a list it did not read. Write the list in that form, or teach this guard the new form in the same PR, with a test for it.".to_string()));
}

#[test]
fn matrix_failures_explain_missing_members_and_extra_entries() {
    assert_eq!(
        matrix_failures(
            &matrix_names(&["z", "a", "y"]),
            &matrix_names(&["c", "a", "b"])
        ),
        vec![
            "[\"b\", \"c\"] are workspace members with no entry in the `crate` matrix of .github/workflows/ci.yml, so CI never builds or tests them on their own. Nothing else reports this: the run stays green, its checks simply have no `crate (<member>)` line for them, and the diff that added the member shows no line where the entry is missing. Add each as `- <name>`, in Cargo.toml order, in the PR that adds the member.",
            "[\"y\", \"z\"] are entries in the `crate` matrix of .github/workflows/ci.yml that name no workspace member, so `cargo build -p` fails in those entries on every run, and a crate that was renamed is checked by no entry. Remove or rename each in the PR that removes or renames the member."
        ]
    );
    assert!(matrix_failures(&matrix_names(&["a"]), &matrix_names(&["a"])).is_empty());
}

#[test]
fn ci_matrix_matches_workspace_members() {
    check_workspace(|root| {
        let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
            .expect("CI workflow must be readable or per-crate coverage cannot be checked");
        let entries = crate_matrix_entries(&workflow).unwrap_or_else(|error| panic!("{error}"));
        let failures = matrix_failures(&entries, &workspace_members(root));
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}

fn matrix_failures(entries: &[String], members: &[String]) -> Vec<String> {
    let (missing, extra) = matrix_drift(entries, members);
    let mut failures = Vec::new();
    if !missing.is_empty() {
        failures.push(format!(
            "{missing:?} are workspace members with no entry in the `crate` matrix of \
                 .github/workflows/ci.yml, so CI never builds or tests them on their own. \
                 Nothing else reports this: the run stays green, its checks simply have no \
                 `crate (<member>)` line for them, and the diff that added the member shows no \
                 line where the entry is missing. Add each as `- <name>`, in Cargo.toml order, \
                 in the PR that adds the member."
        ));
    }
    if !extra.is_empty() {
        failures.push(format!(
            "{extra:?} are entries in the `crate` matrix of .github/workflows/ci.yml that \
                 name no workspace member, so `cargo build -p` fails in those entries on every \
                 run, and a crate that was renamed is checked by no entry. Remove or rename \
                 each in the PR that removes or renames the member."
        ));
    }
    failures
}
