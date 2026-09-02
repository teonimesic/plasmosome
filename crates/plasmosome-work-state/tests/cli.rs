use plasmosome_work_state::contract::parse_contract_request;
use plasmosome_work_state::store::{
    BootstrapDocumentCounts, BootstrapOutcome, BootstrapResult, render_bootstrap_human,
};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

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
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let list = Command::new(binary)
        .current_dir(&root)
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
        .current_dir(&root)
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
        .current_dir(&root)
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
        .current_dir(&root)
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
