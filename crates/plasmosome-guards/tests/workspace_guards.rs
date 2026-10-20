use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use plasmosome_guards::check_workspace;

const TESTKIT: &str = "plasmosome-testkit";

const LANES: &str = "plasmosome-lanes";

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
