use plasmosome_work_state::contract::parse_contract_request;
use plasmosome_work_state::store::{
    BootstrapDocumentCounts, BootstrapOutcome, BootstrapResult, compiled_pin_manifest, host_target,
    render_bootstrap_human,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

#[cfg(unix)]
fn run_without_root_file_bypass(command: &mut Command, root: &Path) -> std::process::Output {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};

    if unsafe { libc::geteuid() } == 0 {
        for directory in [
            root.to_path_buf(),
            root.join("tools"),
            root.join("fake-bin"),
            root.join("common"),
            root.join("common/plasmosome-work-state"),
            root.join("common/plasmosome-work-state/generations"),
            root.join("common/plasmosome-work-state/generations/generation-safe"),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755))
                .expect("root descriptor fixture must be traversable after privilege drop");
        }
        command.uid(65534).gid(65534);
    }
    command
        .output()
        .expect("nonroot descriptor child must start")
}

#[test]
fn all_and_transport_accept_no_remote_or_credential_arguments() {
    for case in ["all", "transport"] {
        let request =
            parse_contract_request(["contract-test", case, "--archive", "archive", "--bd", "bd"])
                .expect("offline contract command parses");
        assert_eq!(request.case, case);
    }
    for forbidden in ["--github-remote", "--confirm-disposable"] {
        assert_eq!(
            parse_contract_request([
                "contract-test",
                "all",
                "--archive",
                "archive",
                "--bd",
                "bd",
                forbidden,
                "value"
            ])
            .unwrap_err(),
            "invalid_command"
        );
    }
}

#[test]
fn individual_new_cases_require_source_ref_and_all_defaults_to_origin_main() {
    for case in [
        "document-mapping",
        "shadow-parity",
        "local-reads",
        "freshness",
        "combined-freshness",
        "online-sync",
    ] {
        let request = parse_contract_request([
            "contract-test",
            case,
            "--source-ref",
            "origin/main",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ])
        .expect("new document case parses with an explicit source ref");
        assert_eq!(request.source_ref.as_deref(), Some("origin/main"));

        assert_eq!(
            parse_contract_request(["contract-test", case, "--archive", "archive", "--bd", "bd",])
                .unwrap_err(),
            "invalid_command"
        );
        assert_eq!(
            parse_contract_request([
                "contract-test",
                case,
                "--source-ref",
                "origin/main",
                "--source-ref",
                "HEAD",
                "--archive",
                "archive",
                "--bd",
                "bd",
            ])
            .unwrap_err(),
            "invalid_command"
        );
    }

    let aggregate =
        parse_contract_request(["contract-test", "all", "--archive", "archive", "--bd", "bd"])
            .expect("the existing aggregate form remains valid");
    assert_eq!(aggregate.source_ref.as_deref(), Some("origin/main"));

    assert_eq!(
        parse_contract_request([
            "contract-test",
            "transport",
            "--source-ref",
            "origin/main",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ])
        .unwrap_err(),
        "invalid_command"
    );
}

#[test]
fn source_flags_are_unambiguous_and_legacy_forms_stay_unchanged() {
    let aggregate = parse_contract_request([
        "contract-test",
        "all",
        "--source-ref",
        "13c0f68c13743f4db2fb123fef560f3fa12734d1",
        "--archive",
        "archive",
        "--bd",
        "bd",
    ])
    .expect("all accepts one explicit source ref");
    assert_eq!(
        aggregate.source_ref.as_deref(),
        Some("13c0f68c13743f4db2fb123fef560f3fa12734d1")
    );

    for values in [
        vec![
            "contract-test",
            "all",
            "--source-ref",
            "",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "document-mapping",
            "--source-ref",
            "   ",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--source-ref",
            "origin/main",
            "--source-ref",
            "HEAD",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--source-ref",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--archive",
            "first",
            "--archive",
            "second",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--archive",
            "archive",
            "--bd",
            "first",
            "--bd",
            "second",
        ],
    ] {
        assert_eq!(
            parse_contract_request(values).unwrap_err(),
            "invalid_command"
        );
    }
}

#[test]
fn public_reads_need_no_artifact_arguments() {
    let binary = env!("CARGO_BIN_EXE_plasmosome-work-state");
    let fixture = tempdir().unwrap();
    let root = fixture.path();
    let source_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::create_dir(root.join("tools")).unwrap();
    fs::copy(
        source_root.join("tools/work-state-beads-1.1.2.toml"),
        root.join("tools/work-state-beads-1.1.2.toml"),
    )
    .unwrap();
    let initialized = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(initialized.status.success());
    let list = Command::new(binary)
        .current_dir(root)
        .arg("list")
        .output()
        .unwrap();
    assert_eq!(list.status.code(), Some(1));
    assert!(String::from_utf8(list.stdout).unwrap().is_empty());
    assert!(
        String::from_utf8(list.stderr)
            .unwrap()
            .contains("not_initialized")
    );

    let forbidden = Command::new(binary)
        .current_dir(root)
        .args(["ready", "--archive", "artifact"])
        .output()
        .unwrap();
    assert_eq!(forbidden.status.code(), Some(2));
    assert!(String::from_utf8(forbidden.stdout).unwrap().is_empty());
    assert!(
        String::from_utf8(forbidden.stderr)
            .unwrap()
            .contains("invalid_command")
    );

    let show = Command::new(binary)
        .current_dir(root)
        .args(["show", "014"])
        .output()
        .unwrap();
    assert_eq!(show.status.code(), Some(2));
    assert!(
        String::from_utf8(show.stderr)
            .unwrap()
            .contains("invalid_document_key")
    );

    let bootstrap = Command::new(binary)
        .current_dir(root)
        .args([
            "bootstrap",
            "--source-ref",
            "origin/main",
            "--archive",
            "/missing/archive",
            "--bd",
            "/missing/bd",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(bootstrap.status.code(), Some(1));
    assert!(String::from_utf8(bootstrap.stdout).unwrap().is_empty());
    assert!(
        String::from_utf8(bootstrap.stderr)
            .unwrap()
            .contains("beads_checksum_mismatch")
    );
}

#[test]
fn sync_cli_accepts_only_optional_json() {
    let binary = env!("CARGO_BIN_EXE_plasmosome-work-state");
    let fixture = tempdir().unwrap();
    let initialized = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert!(initialized.status.success());

    for forbidden in [
        vec!["sync", "--remote", "origin"],
        vec!["sync", "--source-ref", "origin/main"],
        vec!["sync", "--archive", "/artifact"],
        vec!["sync", "--bd", "/bd"],
        vec!["sync", "--token", "secret"],
        vec!["sync", "--json", "--json"],
        vec!["sync", "unexpected"],
    ] {
        let output = Command::new(binary)
            .current_dir(fixture.path())
            .args(&forbidden)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{forbidden:?}");
        assert!(output.stdout.is_empty(), "{forbidden:?}");
        let expected = if forbidden.contains(&"--json") {
            b"{\"code\":\"invalid_command\"}\n".as_slice()
        } else {
            b"error[invalid_command]: invalid_command\n".as_slice()
        };
        assert_eq!(output.stderr, expected, "{forbidden:?}");
    }

    let human = Command::new(binary)
        .current_dir(fixture.path())
        .arg("sync")
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(1));
    assert!(human.stdout.is_empty());
    assert_eq!(
        human.stderr,
        b"error[not_initialized]: not_initialized state_changed=false\n"
    );

    let json = Command::new(binary)
        .current_dir(fixture.path())
        .args(["sync", "--json"])
        .output()
        .unwrap();
    assert_eq!(json.status.code(), Some(1));
    assert!(json.stdout.is_empty());
    assert_eq!(
        json.stderr,
        b"{\"code\":\"not_initialized\",\"state_changed\":false}\n"
    );
}

#[test]
fn bootstrap_source_ref_syntax_refuses_as_invalid_source_ref() {
    let binary = env!("CARGO_BIN_EXE_plasmosome-work-state");

    for (label, source_ref) in [
        ("blank", ""),
        ("whitespace", " \t "),
        ("carriage return", "origin/main\r"),
        ("line feed", "origin/main\n"),
    ] {
        let json = Command::new(binary)
            .args([
                "bootstrap",
                "--source-ref",
                source_ref,
                "--archive",
                "/missing/archive",
                "--bd",
                "/missing/bd",
                "--json",
            ])
            .output()
            .unwrap();
        assert_eq!(json.status.code(), Some(2), "{label}");
        assert!(json.stdout.is_empty(), "{label}");
        assert_eq!(
            json.stderr, b"{\"code\":\"invalid_source_ref\"}\n",
            "{label}"
        );

        let human = Command::new(binary)
            .args([
                "bootstrap",
                "--source-ref",
                source_ref,
                "--archive",
                "/missing/archive",
                "--bd",
                "/missing/bd",
            ])
            .output()
            .unwrap();
        assert_eq!(human.status.code(), Some(2), "{label}");
        assert!(human.stdout.is_empty(), "{label}");
        assert_eq!(
            human.stderr, b"error[invalid_source_ref]: invalid_source_ref\n",
            "{label}"
        );
    }
}

#[test]
fn bootstrap_human_output_reports_the_same_evidence_as_json() {
    let result = BootstrapResult {
        outcome: BootstrapOutcome::Installed,
        source_commit: "a".repeat(40),
        local_generation: "local-generation".into(),
        document_counts: BootstrapDocumentCounts {
            intent: 1,
            spec: 2,
            task: 3,
        },
        logical_export_sha256: "b".repeat(64),
    };
    let human = render_bootstrap_human(&result);
    let json = serde_json::to_value(&result).unwrap();

    assert_eq!(json["document_counts"]["intent"], 1);
    assert_eq!(json["document_counts"]["spec"], 2);
    assert_eq!(json["document_counts"]["task"], 3);
    for expected in [
        "bootstrap: installed",
        result.source_commit.as_str(),
        result.local_generation.as_str(),
        "document counts: intent=1 spec=2 task=3",
        result.logical_export_sha256.as_str(),
    ] {
        assert!(human.contains(expected), "missing {expected} from {human}");
    }
}

#[test]
fn ordinary_launcher_executes_installed_wrapper_without_cargo() {
    let root = tempdir().unwrap();
    let tools = root.path().join("tools");
    let fake_bin = root.path().join("fake-bin");
    let common = root.path().join("common");
    let generation = common.join("plasmosome-work-state/generations/generation-safe");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&generation).unwrap();
    fs::write(
        common.join("plasmosome-work-state/current"),
        "generation-safe\n",
    )
    .unwrap();
    let launcher = tools.join("work-state");
    fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/work-state"),
        &launcher,
    )
    .unwrap();
    let wrapper = generation.join("plasmosome-work-state");
    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nprintf 'wrapper:%s\\n' \"$*\"\n",
    )
    .unwrap();
    let git = fake_bin.join("git");
    let canonical_root = root.path().canonicalize().unwrap();
    let canonical_common = common.canonicalize().unwrap();
    fs::write(
        &git,
        format!(
            "#!/usr/bin/env bash\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
            canonical_root.display(),
            canonical_common.display()
        ),
    )
    .unwrap();
    let cargo = fake_bin.join("cargo");
    fs::write(
        &cargo,
        format!(
            "#!/usr/bin/env bash\nprintf cargo > '{}'\nexit 97\n",
            root.path().join("cargo-ran").display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for path in [&launcher, &wrapper, &git, &cargo] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
    let output = Command::new(&launcher)
        .current_dir(root.path())
        .env("PATH", path)
        .arg("list")
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "wrapper:list\n");
    assert!(!root.path().join("cargo-ran").exists());
}

#[cfg(unix)]
#[test]
fn sync_launcher_executes_only_the_installed_wrapper() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let tools = root.path().join("tools");
    let fake_bin = root.path().join("fake-bin");
    let common = root.path().join("common");
    let generation = common.join("plasmosome-work-state/generations/generation-safe");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&generation).unwrap();
    fs::write(
        common.join("plasmosome-work-state/current"),
        "generation-safe\n",
    )
    .unwrap();
    let launcher = tools.join("work-state");
    fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/work-state"),
        &launcher,
    )
    .unwrap();
    let wrapper = generation.join("plasmosome-work-state");
    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nprintf 'wrapper:%s\\n' \"$*\"\n",
    )
    .unwrap();
    let canonical_root = root.path().canonicalize().unwrap();
    let canonical_common = common.canonicalize().unwrap();
    let git = fake_bin.join("git");
    fs::write(
        &git,
        format!(
            "#!/usr/bin/env bash\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
            canonical_root.display(),
            canonical_common.display()
        ),
    )
    .unwrap();
    let cargo = fake_bin.join("cargo");
    fs::write(
        &cargo,
        format!(
            "#!/usr/bin/env bash\nprintf cargo > '{}'\nexit 97\n",
            root.path().join("cargo-ran").display()
        ),
    )
    .unwrap();
    for path in [&launcher, &wrapper, &git, &cargo] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
    let output = Command::new(&launcher)
        .current_dir(root.path())
        .env("PATH", path)
        .args(["sync", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"wrapper:sync --json\n");
    assert!(output.stderr.is_empty());
    assert!(
        !root.path().join("cargo-ran").exists(),
        "the installed sync route must never invoke Cargo"
    );
}

#[cfg(all(
    unix,
    any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    )
))]
#[test]
fn copied_generation_a_wrapper_uses_a_after_current_points_to_b() {
    use std::os::unix::fs::PermissionsExt;

    fn write_manifest(
        generation: &std::path::Path,
        wrapper_sha256: String,
        beads_binary_sha256: &str,
        target: &str,
    ) {
        fs::write(
            generation.join("state.json"),
            serde_json::json!({
                "schema_version": 1,
                "authority_mode": "markdown-shadow",
                "source_commit": "a".repeat(40),
                "logical_export_sha256": "b".repeat(64),
                "operational_projection_sha256": "c".repeat(64),
                "local_generation": "d".repeat(40),
                "host_target": target,
                "wrapper_sha256": wrapper_sha256,
                "beads_binary_sha256": beads_binary_sha256,
                "remote_relation": "unknown",
                "remote_generation": serde_json::Value::Null,
                "remote_observed_at": serde_json::Value::Null,
                "observed_local_generation": serde_json::Value::Null,
                "last_successful_sync_at": serde_json::Value::Null,
                "pending_operation_ids": [],
            })
            .to_string(),
        )
        .unwrap();
    }

    let root = tempdir().unwrap();
    let fake_bin = root.path().join("fake-bin");
    let common = root.path().join("common");
    let state = common.join("plasmosome-work-state");
    let generations = state.join("generations");
    let generation_a = generations.join("generation-a");
    let generation_b = generations.join("generation-b");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&generation_a).unwrap();
    fs::create_dir_all(&generation_b).unwrap();
    fs::write(state.join("current"), "generation-b\n").unwrap();

    let wrapper_a = generation_a.join("plasmosome-work-state");
    fs::copy(env!("CARGO_BIN_EXE_plasmosome-work-state"), &wrapper_a).unwrap();
    fs::set_permissions(&wrapper_a, fs::Permissions::from_mode(0o700)).unwrap();
    let wrapper_b = generation_b.join("plasmosome-work-state");
    fs::write(&wrapper_b, "different retained wrapper").unwrap();
    let host_target = host_target();
    let target = compiled_pin_manifest()
        .unwrap()
        .targets
        .into_iter()
        .find(|candidate| candidate.target == host_target)
        .unwrap();
    write_manifest(
        &generation_a,
        format!("{:x}", Sha256::digest(fs::read(&wrapper_a).unwrap())),
        &target.binary_sha256,
        host_target,
    );
    write_manifest(
        &generation_b,
        format!("{:x}", Sha256::digest(fs::read(&wrapper_b).unwrap())),
        &target.binary_sha256,
        host_target,
    );
    for directory in ["home", "xdg_config", "xdg_cache", "xdg_data", "tmp"] {
        fs::create_dir_all(generation_a.join("runtime").join(directory)).unwrap();
    }
    fs::write(generation_a.join("runtime/git_config_global"), "").unwrap();

    let canonical_root = root.path().canonicalize().unwrap();
    let canonical_common = common.canonicalize().unwrap();
    let git = fake_bin.join("git");
    fs::write(
        &git,
        format!(
            "#!/bin/sh\nif [ \"$2\" = \"--show-toplevel\" ]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
            canonical_root.display(),
            canonical_common.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());

    let output = Command::new(&wrapper_a)
        .current_dir(root.path())
        .env("PATH", path)
        .arg("list")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stdout).unwrap().is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error[installed_beads_missing]: installed_beads_missing\n"
    );
}

#[cfg(unix)]
#[test]
fn ordinary_launcher_refusals_emit_one_stable_error_without_underlying_diagnostics() {
    use std::os::unix::fs::PermissionsExt;

    fn launcher_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let root = tempdir().unwrap();
        let tools = root.path().join("tools");
        let fake_bin = root.path().join("fake-bin");
        let common = root.path().join("common");
        let state = common.join("plasmosome-work-state");
        let generation = state.join("generations/generation-safe");
        fs::create_dir_all(&tools).unwrap();
        fs::create_dir_all(&fake_bin).unwrap();
        fs::create_dir_all(&generation).unwrap();
        fs::write(state.join("current"), "generation-safe\n").unwrap();
        let launcher = tools.join("work-state");
        fs::copy(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/work-state"),
            &launcher,
        )
        .unwrap();
        let wrapper = generation.join("plasmosome-work-state");
        fs::write(
            &wrapper,
            "#!/usr/bin/env bash\nprintf 'wrapper:%s\\n' \"$*\"\n",
        )
        .unwrap();
        let git = fake_bin.join("git");
        let canonical_root = root.path().canonicalize().unwrap();
        let canonical_common = common.canonicalize().unwrap();
        fs::write(
            &git,
            format!(
                "#!/usr/bin/env bash\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
                canonical_root.display(),
                canonical_common.display()
            ),
        )
        .unwrap();
        for path in [&launcher, &wrapper, &git] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        (root, launcher, fake_bin, state, wrapper)
    }

    fn assert_json_refusal(output: std::process::Output, label: &str, code: &str) {
        assert_eq!(output.status.code(), Some(1), "{label}");
        assert!(
            String::from_utf8(output.stdout).unwrap().is_empty(),
            "{label}"
        );
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("{{\"code\":\"{code}\"}}\n"),
            "{label}",
        );
    }

    let (root, launcher, fake_bin, state, wrapper) = launcher_fixture();
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());

    fs::write(
        fake_bin.join("git"),
        "#!/usr/bin/env bash\nprintf 'untrusted git diagnostic\\n' >&2\nexit 71\n",
    )
    .unwrap();
    fs::set_permissions(fake_bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_json_refusal(
        Command::new(&launcher)
            .current_dir(root.path())
            .env("PATH", &path)
            .args(["list", "--json"])
            .output()
            .unwrap(),
        "git failure",
        "invalid_store_location",
    );

    assert_json_refusal(
        Command::new("/bin/bash")
            .arg(&launcher)
            .current_dir(root.path())
            .env_clear()
            .env("PATH", "/definitely-missing")
            .args(["list", "--json"])
            .output()
            .unwrap(),
        "missing PATH",
        "invalid_store_location",
    );

    let canonical_root = root.path().canonicalize().unwrap();
    let canonical_common = state.parent().unwrap().canonicalize().unwrap();
    fs::write(
        fake_bin.join("git"),
        format!(
            "#!/usr/bin/env bash\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
            canonical_root.display(),
            canonical_common.display()
        ),
    )
    .unwrap();
    fs::set_permissions(fake_bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();

    fs::set_permissions(state.join("current"), fs::Permissions::from_mode(0o000)).unwrap();
    let mut descriptor_failure = Command::new(&launcher);
    descriptor_failure
        .current_dir(root.path())
        .env("PATH", &path)
        .args(["list", "--json"]);
    assert_json_refusal(
        run_without_root_file_bypass(&mut descriptor_failure, root.path()),
        "current descriptor failure",
        "invalid_store",
    );
    fs::set_permissions(state.join("current"), fs::Permissions::from_mode(0o644)).unwrap();

    fs::write(
        state.join("current"),
        "generation-safe\nunterminated-suffix",
    )
    .unwrap();
    assert_json_refusal(
        Command::new(&launcher)
            .current_dir(root.path())
            .env("PATH", &path)
            .args(["list", "--json"])
            .output()
            .unwrap(),
        "unterminated current suffix",
        "invalid_store",
    );
    fs::write(state.join("current"), "generation-safe\n").unwrap();

    fs::write(state.join("current"), b"generation-safe\0\n").unwrap();
    assert_json_refusal(
        Command::new(&launcher)
            .current_dir(root.path())
            .env("PATH", &path)
            .args(["list", "--json"])
            .output()
            .unwrap(),
        "embedded current NUL",
        "invalid_store",
    );
    fs::write(state.join("current"), "generation-safe\n").unwrap();

    let calls = root.path().join("git-calls");
    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\ngit rev-parse --show-toplevel >/dev/null\nprintf 'wrapper:%s\\n' \"$*\"\n",
    )
    .unwrap();
    fs::write(
        fake_bin.join("git"),
        format!(
            "#!/usr/bin/env bash\ncount=0\nif [[ -f '{calls}' ]]; then read -r count < '{calls}'; fi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > '{calls}'\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then\n  if [[ \"$count\" -eq 1 ]]; then printf '%s\\n\\n' '{root}'; else printf '%s\\n' '{root}'; fi\nelse\n  printf '%s\\n' '{common}'\nfi\n",
            calls = calls.display(),
            root = canonical_root.display(),
            common = canonical_common.display(),
        ),
    )
    .unwrap();
    for path in [&wrapper, &fake_bin.join("git")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_json_refusal(
        Command::new(&launcher)
            .current_dir(root.path())
            .env("PATH", &path)
            .args(["list", "--json"])
            .output()
            .unwrap(),
        "multiline outer locator",
        "invalid_store_location",
    );
    assert_eq!(fs::read_to_string(&calls).unwrap(), "2\n");

    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nprintf '{\"code\":\"invalid_store\"}\\nuntrusted suffix\\n' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    assert_json_refusal(
        Command::new(&launcher)
            .current_dir(root.path())
            .env("PATH", &path)
            .args(["list", "--json"])
            .output()
            .unwrap(),
        "multiline wrapper refusal",
        "invalid_store",
    );

    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nprintf '{\"code\":\"invalid_store\"}\\n' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let human_wrapper_refusal = Command::new(&launcher)
        .current_dir(root.path())
        .env("PATH", &path)
        .arg("list")
        .output()
        .unwrap();
    assert_eq!(human_wrapper_refusal.status.code(), Some(1));
    assert!(
        String::from_utf8(human_wrapper_refusal.stdout)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        String::from_utf8(human_wrapper_refusal.stderr).unwrap(),
        "error[invalid_store]: invalid_store\n"
    );

    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nprintf 'error[document_not_found]: document_not_found (task:999)\\n' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let canonical_human_refusal = Command::new(&launcher)
        .current_dir(root.path())
        .env("PATH", &path)
        .arg("list")
        .output()
        .unwrap();
    assert_eq!(canonical_human_refusal.status.code(), Some(1));
    assert!(
        String::from_utf8(canonical_human_refusal.stdout)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        String::from_utf8(canonical_human_refusal.stderr).unwrap(),
        "error[document_not_found]: document_not_found (task:999)\n"
    );

    fs::write(&wrapper, [0_u8, 1, 2, 3]).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    assert_json_refusal(
        Command::new(&launcher)
            .current_dir(root.path())
            .env("PATH", path)
            .args(["list", "--json"])
            .output()
            .unwrap(),
        "wrapper exec failure",
        "invalid_store",
    );
}

#[cfg(unix)]
#[test]
fn ordinary_launcher_clears_sensitive_environment_before_wrapper_execution() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let tools = root.path().join("tools");
    let fake_bin = root.path().join("fake-bin");
    let common = root.path().join("common");
    let state = common.join("plasmosome-work-state");
    let generation = state.join("generations/generation-safe");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&generation).unwrap();
    fs::write(state.join("current"), "generation-safe\n").unwrap();
    let launcher = tools.join("work-state");
    fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/work-state"),
        &launcher,
    )
    .unwrap();
    let wrapper = generation.join("plasmosome-work-state");
    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nfor variable in GITHUB_TOKEN SSH_AUTH_SOCK HTTPS_PROXY HTTP_PROXY ALL_PROXY AWS_ACCESS_KEY_ID GIT_CONFIG_GLOBAL; do\n  if [[ -n ${!variable+x} ]]; then\n    printf 'leaked:%s=%s\\n' \"$variable\" \"${!variable}\"\n    exit 97\n  fi\ndone\nprintf 'path:%s\\n' \"$PATH\"\n",
    )
    .unwrap();
    let git = fake_bin.join("git");
    let canonical_root = root.path().canonicalize().unwrap();
    let canonical_common = common.canonicalize().unwrap();
    fs::write(
        &git,
        format!(
            "#!/usr/bin/env bash\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
            canonical_root.display(),
            canonical_common.display()
        ),
    )
    .unwrap();
    for path in [&launcher, &wrapper, &git] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());

    let output = Command::new(&launcher)
        .current_dir(root.path())
        .env("PATH", &path)
        .env("GITHUB_TOKEN", "secret-github")
        .env("SSH_AUTH_SOCK", "/secret/ssh-agent")
        .env("HTTPS_PROXY", "https://secret.proxy")
        .env("HTTP_PROXY", "http://secret.proxy")
        .env("ALL_PROXY", "socks5://secret.proxy")
        .env("AWS_ACCESS_KEY_ID", "secret-cloud")
        .env("GIT_CONFIG_GLOBAL", "/secret/gitconfig")
        .arg("list")
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !stdout.contains("leaked:"),
        "the wrapper received a caller-provided sensitive value: {stdout}"
    );
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("path:{path}\n"));
}

#[cfg(unix)]
#[test]
fn bootstrap_launcher_uses_release_locked_offline_cargo() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let tools = root.path().join("tools");
    let fake_bin = root.path().join("fake-bin");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();

    let launcher = tools.join("work-state");
    fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/work-state"),
        &launcher,
    )
    .unwrap();
    let record = root.path().join("cargo-arguments");
    let cargo = fake_bin.join("cargo");
    fs::write(
        &cargo,
        format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > '{}'\n",
            record.display()
        ),
    )
    .unwrap();
    for path in [&launcher, &cargo] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());

    let bootstrap = Command::new(&launcher)
        .current_dir(root.path())
        .env("PATH", &path)
        .args([
            "bootstrap",
            "--source-ref",
            "origin/main",
            "--archive",
            "archive",
            "--bd",
            "bd",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(
        bootstrap.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&bootstrap.stderr)
    );
    assert_eq!(
        fs::read_to_string(&record).unwrap(),
        "run\n--release\n--locked\n--offline\n--quiet\n-p\nplasmosome-work-state\n--\nbootstrap\n--source-ref\norigin/main\n--archive\narchive\n--bd\nbd\n--json\n"
    );

    let contract = Command::new(&launcher)
        .current_dir(root.path())
        .env("PATH", path)
        .args(["contract-test", "all", "--archive", "archive", "--bd", "bd"])
        .output()
        .unwrap();
    assert_eq!(
        contract.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&contract.stderr)
    );
    assert_eq!(
        fs::read_to_string(record).unwrap(),
        "run\n--locked\n--offline\n--quiet\n-p\nplasmosome-work-state\n--\ncontract-test\nall\n--archive\narchive\n--bd\nbd\n"
    );
}

#[cfg(unix)]
#[test]
fn ordinary_launcher_refuses_a_symlinked_generation_component() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = tempdir().unwrap();
    let tools = root.path().join("tools");
    let fake_bin = root.path().join("fake-bin");
    let common = root.path().join("common");
    let state = common.join("plasmosome-work-state");
    let redirected_generations = root.path().join("redirected-generations");
    let generation = redirected_generations.join("generation-safe");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::create_dir_all(&generation).unwrap();
    fs::write(state.join("current"), "generation-safe\n").unwrap();
    symlink(&redirected_generations, state.join("generations")).unwrap();

    let launcher = tools.join("work-state");
    fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/work-state"),
        &launcher,
    )
    .unwrap();
    let wrapper = generation.join("plasmosome-work-state");
    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nprintf 'unexpected wrapper execution\\n'\n",
    )
    .unwrap();
    let git = fake_bin.join("git");
    let canonical_root = root.path().canonicalize().unwrap();
    let canonical_common = common.canonicalize().unwrap();
    fs::write(
        &git,
        format!(
            "#!/usr/bin/env bash\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
            canonical_root.display(),
            canonical_common.display()
        ),
    )
    .unwrap();
    for path in [&launcher, &wrapper, &git] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = Command::new(&launcher)
        .current_dir(root.path())
        .env(
            "PATH",
            format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap()),
        )
        .arg("list")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stdout).unwrap().is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("invalid_store")
    );
}

#[cfg(unix)]
#[test]
fn ordinary_launcher_does_not_confuse_a_symlinked_state_root_with_an_uninitialized_store() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = tempdir().unwrap();
    let tools = root.path().join("tools");
    let fake_bin = root.path().join("fake-bin");
    let common = root.path().join("common");
    let state = common.join("plasmosome-work-state");
    let redirected_state = root.path().join("redirected-state");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    fs::create_dir_all(&common).unwrap();
    fs::create_dir_all(&redirected_state).unwrap();
    symlink(&redirected_state, &state).unwrap();

    let launcher = tools.join("work-state");
    fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/work-state"),
        &launcher,
    )
    .unwrap();
    let git = fake_bin.join("git");
    let canonical_root = root.path().canonicalize().unwrap();
    let canonical_common = common.canonicalize().unwrap();
    fs::write(
        &git,
        format!(
            "#!/usr/bin/env bash\nif [[ \"$*\" == *\"--show-toplevel\"* ]]; then printf '%s\\n' '{}'; else printf '%s\\n' '{}'; fi\n",
            canonical_root.display(),
            canonical_common.display()
        ),
    )
    .unwrap();
    for path in [&launcher, &git] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = Command::new(&launcher)
        .current_dir(root.path())
        .env(
            "PATH",
            format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap()),
        )
        .arg("list")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stdout).unwrap().is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("invalid_store")
    );
}
