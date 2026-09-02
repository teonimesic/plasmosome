use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use plasmosome_work_state::command::{CommandOutput, CommandSpec, RecordingCommandRunner};
use plasmosome_work_state::document::parse_document;
use plasmosome_work_state::pin::PinManifest;
use plasmosome_work_state::shadow::{
    canonical_logical_export, canonical_operational_projection, initial_operational_metadata,
    logical_export_digest, operational_projection_digest, to_operational_beads_jsonl,
};
use plasmosome_work_state::store::{
    ActivationFault, BootstrapLock, BootstrapRequest, CurrentGeneration, StateManifest,
    activate_staged_generation, active_generation, bootstrap, current_generation, locate_store,
    read_disposable_snapshot, validate_bootstrap_command, validate_fenced_snapshot,
    validate_read_command, validate_read_locator_command,
};
use tempfile::tempdir;

fn git_output(value: impl Into<String>) -> Result<CommandOutput, String> {
    Ok(CommandOutput::success(value))
}

fn locate(
    checkout: &Path,
    top_level: &Path,
    common_dir: &Path,
) -> Result<plasmosome_work_state::store::StoreLocation, String> {
    let top_level = top_level.canonicalize().unwrap();
    let common_dir = common_dir.canonicalize().unwrap();
    let mut runner = RecordingCommandRunner::scripted(vec![
        git_output(format!("{}\n", top_level.display())),
        git_output(format!("{}\n", common_dir.display())),
    ]);
    let result = locate_store(&mut runner, checkout, BTreeMap::new())
        .map_err(|error| error.code().to_owned());
    assert!(runner.finish().is_ok(), "{result:?}");
    result
}

fn valid_manifest() -> String {
    serde_json::json!({
        "schema_version": 1,
        "authority_mode": "markdown-shadow",
        "source_commit": "a".repeat(40),
        "logical_export_sha256": "b".repeat(64),
        "operational_projection_sha256": "c".repeat(64),
        "local_generation": "d".repeat(40),
        "host_target": "aarch64-apple-darwin",
        "wrapper_sha256": "e".repeat(64),
        "beads_binary_sha256": "f".repeat(64),
        "remote_relation": "unknown",
        "remote_generation": null,
        "remote_observed_at": null,
        "observed_local_generation": null,
        "last_successful_sync_at": null,
        "pending_operation_ids": []
    })
    .to_string()
}

fn disposable_environment(root: &Path) -> BTreeMap<String, String> {
    let runtime = root.join("runtime");
    for directory in ["home", "xdg_config", "xdg_cache", "xdg_data", "tmp"] {
        fs::create_dir_all(runtime.join(directory)).unwrap();
    }
    fs::write(runtime.join("git_config_global"), "").unwrap();
    let mut environment = BTreeMap::from([
        ("HOME".into(), runtime.join("home").display().to_string()),
        (
            "XDG_CONFIG_HOME".into(),
            runtime.join("xdg_config").display().to_string(),
        ),
        (
            "XDG_CACHE_HOME".into(),
            runtime.join("xdg_cache").display().to_string(),
        ),
        (
            "XDG_DATA_HOME".into(),
            runtime.join("xdg_data").display().to_string(),
        ),
        ("TMPDIR".into(), runtime.join("tmp").display().to_string()),
        (
            "GIT_CONFIG_GLOBAL".into(),
            runtime.join("git_config_global").display().to_string(),
        ),
    ]);
    for (key, value) in [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("GIT_OPTIONAL_LOCKS", "0"),
        ("BD_DISABLE_METRICS", "1"),
        ("BD_DISABLE_EVENT_FLUSH", "1"),
        ("BD_NON_INTERACTIVE", "1"),
        ("CI", "true"),
    ] {
        environment.insert(key.into(), value.into());
    }
    environment.insert("PATH".into(), std::env::var("PATH").unwrap());
    environment
}

#[test]
fn linked_worktrees_resolve_one_common_store() {
    let root = tempdir().unwrap();
    let checkout_a = root.path().join("checkout-a");
    let checkout_b = root.path().join("checkout-b");
    let common = root.path().join("common");
    let other_checkout = root.path().join("other-checkout");
    let other_common = root.path().join("other-common");
    for path in [
        &checkout_a,
        &checkout_b,
        &common,
        &other_checkout,
        &other_common,
    ] {
        fs::create_dir_all(path).unwrap();
    }

    let first = locate(&checkout_a, &checkout_a, &common).unwrap();
    let second = locate(&checkout_b, &checkout_b, &common).unwrap();
    let other = locate(&other_checkout, &other_checkout, &other_common).unwrap();

    assert_eq!(first.common_dir, second.common_dir);
    assert_eq!(first.state_root, second.state_root);
    assert_ne!(first.state_root, other.state_root);

    let canonical_checkout = checkout_a.canonicalize().unwrap();
    let canonical_common = common.canonicalize().unwrap();
    for (top_level, common_dir, command_count) in [
        (
            "relative".to_owned(),
            canonical_common.display().to_string(),
            1,
        ),
        (
            canonical_checkout.display().to_string(),
            "relative".into(),
            2,
        ),
        (
            format!("{}\nother", canonical_checkout.display()),
            canonical_common.display().to_string(),
            1,
        ),
        (
            canonical_checkout.display().to_string(),
            format!("{}\nother", canonical_common.display()),
            2,
        ),
        (
            root.path().join("missing").display().to_string(),
            canonical_common.display().to_string(),
            1,
        ),
    ] {
        let mut runner = RecordingCommandRunner::scripted(vec![
            git_output(format!("{top_level}\n")),
            git_output(format!("{common_dir}\n")),
        ]);
        let error = locate_store(&mut runner, &checkout_a, BTreeMap::new()).unwrap_err();
        assert_eq!(error.code(), "invalid_store_location");
        assert_eq!(runner.commands().len(), command_count);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let redirected = root.path().join("redirected-common");
        symlink(&common, &redirected).unwrap();
        let mut runner = RecordingCommandRunner::scripted(vec![
            git_output(format!("{}\n", canonical_checkout.display())),
            git_output(format!("{}\n", redirected.display())),
        ]);
        let error = locate_store(&mut runner, &checkout_a, BTreeMap::new()).unwrap_err();
        assert_eq!(error.code(), "invalid_store_location");
        assert!(runner.finish().is_ok());
    }
}

#[test]
fn current_pointer_and_manifest_refuse_unsafe_layouts() {
    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(&location.state_root).unwrap();
    let generation = location.generations_dir.join("generation-safe");
    fs::create_dir_all(&generation).unwrap();
    fs::write(location.state_root.join("current"), "generation-safe\n").unwrap();
    fs::write(generation.join("state.json"), valid_manifest()).unwrap();

    let loaded = current_generation(&location).expect("safe generation loads");
    assert_eq!(loaded.name, "generation-safe");
    assert_eq!(loaded.manifest.source_commit, "a".repeat(40));

    for pointer in [
        "generation-safe",
        "../generation-safe\n",
        "/tmp/generation-safe\n",
        "generation-safe\nextra\n",
    ] {
        fs::write(location.state_root.join("current"), pointer).unwrap();
        let error = current_generation(&location).unwrap_err();
        assert_eq!(error.code(), "invalid_store");
    }
    fs::write(location.state_root.join("current"), "generation-safe\n").unwrap();
    fs::write(generation.join("state.json"), "{\"schema_version\":1}").unwrap();
    let error = current_generation(&location).unwrap_err();
    assert_eq!(error.code(), "invalid_store");
}

#[cfg(unix)]
#[test]
fn current_generation_refuses_symlinked_shared_state_components() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    let redirected = root.path().join("redirected-state");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    fs::create_dir_all(&redirected).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(redirected.join("generations/generation-safe")).unwrap();
    fs::write(redirected.join("current"), "generation-safe\n").unwrap();
    fs::write(
        redirected.join("generations/generation-safe/state.json"),
        valid_manifest(),
    )
    .unwrap();
    symlink(&redirected, &location.state_root).unwrap();

    assert_eq!(
        current_generation(&location).unwrap_err().code(),
        "invalid_store"
    );
}

#[cfg(unix)]
#[test]
fn active_generation_does_not_confuse_a_symlinked_state_root_with_an_uninitialized_store() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    let redirected = root.path().join("redirected-state");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    fs::create_dir_all(&redirected).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    symlink(&redirected, &location.state_root).unwrap();

    assert_eq!(
        active_generation(&location).unwrap_err().code(),
        "invalid_store",
        "a symlinked state root is corrupt even when it has no current pointer"
    );
}

#[cfg(unix)]
#[test]
fn ordinary_reads_refuse_a_symlinked_runtime_component() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let generation_root = root.path().join("generation");
    let redirected_runtime = root.path().join("redirected-runtime");
    fs::create_dir_all(&generation_root).unwrap();
    for directory in ["home", "xdg_config", "xdg_cache", "xdg_data", "tmp"] {
        fs::create_dir_all(redirected_runtime.join(directory)).unwrap();
    }
    fs::write(redirected_runtime.join("git_config_global"), "").unwrap();
    symlink(&redirected_runtime, generation_root.join("runtime")).unwrap();
    fs::write(generation_root.join("bd"), "unverified fixture binary").unwrap();
    let mut manifest: StateManifest = serde_json::from_str(&valid_manifest()).unwrap();
    manifest.beads_binary_sha256 = "d".repeat(64);
    let generation = CurrentGeneration {
        name: "generation-safe".into(),
        root: generation_root,
        manifest,
    };
    let pin = PinManifest::parse(&format!(
        "version = \"1.1.2\"\nrelease = \"https://example.invalid/release\"\nsource_commit = \"{}\"\nlicense = \"MIT\"\nchecksums_url = \"https://example.invalid/checksums\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"aarch64-apple-darwin\"\narchive = \"beads_1.1.2_darwin_arm64.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
        "a".repeat(40),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    ))
    .unwrap();
    let mut runner = RecordingCommandRunner::default();

    assert_eq!(
        read_disposable_snapshot(&mut runner, &generation, &pin, "aarch64-apple-darwin")
            .unwrap_err()
            .code(),
        "invalid_store"
    );
    assert!(runner.commands().is_empty());
}

#[test]
fn bootstrap_activation_survives_every_interruption_boundary() {
    for fault in [
        ActivationFault::BeforeGenerationRename,
        ActivationFault::BeforePointerWrite,
        ActivationFault::BeforePointerRename,
    ] {
        let root = tempdir().unwrap();
        let checkout = root.path().join("checkout");
        let common = root.path().join("common");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&common).unwrap();
        let location = locate(&checkout, &checkout, &common).unwrap();
        fs::create_dir_all(&location.generations_dir).unwrap();
        let old = location.generations_dir.join("generation-old");
        fs::create_dir_all(&old).unwrap();
        fs::write(location.state_root.join("current"), "generation-old\n").unwrap();
        let staging = location.generations_dir.join(".staging-new");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("complete"), "complete generation").unwrap();

        let error = activate_staged_generation(&location, &staging, "generation-new", Some(fault))
            .unwrap_err();
        assert_eq!(error.code(), "bootstrap_interrupted");

        let current = fs::read_to_string(location.state_root.join("current")).unwrap();
        assert!(matches!(
            current.as_str(),
            "generation-old\n" | "generation-new\n"
        ));
        if current == "generation-new\n" {
            assert_eq!(
                fs::read_to_string(location.generations_dir.join("generation-new/complete"))
                    .unwrap(),
                "complete generation"
            );
        }
    }
}

#[test]
fn activation_pointer_staging_uses_an_unpredictable_safe_name() {
    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(&location.generations_dir).unwrap();
    fs::write(location.state_root.join("current"), "generation-old\n").unwrap();
    let staging = location.generations_dir.join(".staging-new");
    fs::create_dir_all(&staging).unwrap();

    assert_eq!(
        activate_staged_generation(
            &location,
            &staging,
            "generation-new",
            Some(ActivationFault::BeforePointerRename),
        )
        .unwrap_err()
        .code(),
        "bootstrap_interrupted"
    );
    let pointer_staging = fs::read_dir(&location.state_root)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.file_name().into_string().unwrap())
        .find(|name| name.starts_with(".current-"))
        .expect("interrupted activation retains its same-directory temporary pointer");
    assert!(
        !pointer_staging.contains(&format!("-{}-", std::process::id())),
        "pointer staging names must not disclose the predictable process id"
    );
}

#[test]
fn bootstrap_lock_refuses_contention_without_waiting() {
    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(&location.state_root).unwrap();

    let first = BootstrapLock::acquire(&location).expect("first lock is acquired");
    let error = BootstrapLock::acquire(&location).unwrap_err();
    assert_eq!(error.code(), "bootstrap_busy");
    drop(first);
    BootstrapLock::acquire(&location).expect("process-scoped lock is released on drop");
}

#[cfg(unix)]
#[test]
fn bootstrap_lock_refuses_a_symlinked_state_root() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    let redirected = root.path().join("redirected");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    fs::create_dir_all(&redirected).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    symlink(&redirected, &location.state_root).unwrap();

    assert_eq!(
        BootstrapLock::acquire(&location).unwrap_err().code(),
        "invalid_store"
    );
}

#[test]
fn snapshot_reads_one_unchanged_committed_generation() {
    let mut manifest: StateManifest = serde_json::from_str(&valid_manifest()).unwrap();
    let documents = vec![
        parse_document(
            "docs/intents/001-intent.md",
            "---\nid: 001\ntitle: Intent\nstatus: approved\n---\n",
            &"a".repeat(40),
        )
        .unwrap(),
        parse_document(
            "docs/specs/001-spec.md",
            "---\nid: 001\ntitle: Spec\nstatus: accepted\nintents: [001]\n---\n",
            &"a".repeat(40),
        )
        .unwrap(),
        parse_document(
            "tasks/001-task.md",
            "---\nid: 001\ntitle: Task\nstatus: planned\npriority: 1\nintents: [001]\nspecs: [001]\n---\n",
            &"a".repeat(40),
        )
        .unwrap(),
    ];
    let operational = initial_operational_metadata(&documents).unwrap();
    let export = to_operational_beads_jsonl(&documents, &operational).unwrap();
    manifest.logical_export_sha256 =
        logical_export_digest(&canonical_logical_export(&documents).unwrap());
    let operational_documents =
        plasmosome_work_state::shadow::decode_operational_beads_jsonl(&export).unwrap();
    manifest.operational_projection_sha256 = operational_projection_digest(
        &canonical_operational_projection(&operational_documents).unwrap(),
    );
    let status = serde_json::json!({
        "schema_version": 1,
        "branch": "main",
        "commit": manifest.local_generation,
    })
    .to_string();
    let key_values = serde_json::json!({
        "schema_version": 1,
        "plasmosome.authority-mode": "markdown-shadow",
        "plasmosome.source-commit": manifest.source_commit,
    })
    .to_string();

    let snapshot = validate_fenced_snapshot(&manifest, &status, &export, &key_values, &status)
        .expect("one committed export agrees with both fences and the manifest");
    assert_eq!(snapshot.documents.len(), 3);
    assert_eq!(
        snapshot.freshness.local_generation,
        manifest.local_generation
    );

    let reordered_export = export.lines().rev().collect::<Vec<_>>().join("\n");
    let reordered =
        validate_fenced_snapshot(&manifest, &status, &reordered_export, &key_values, &status)
            .expect("Beads presentation order is normalized to the canonical projection order");
    assert_eq!(
        reordered
            .documents
            .iter()
            .map(|document| document.document.record.document_key.as_str())
            .collect::<Vec<_>>(),
        vec!["intent:001", "spec:001", "task:001"]
    );

    let changed_status = serde_json::json!({
        "schema_version": 1,
        "branch": "main",
        "commit": "later-commit",
    })
    .to_string();
    assert_eq!(
        validate_fenced_snapshot(&manifest, &status, &export, &key_values, &changed_status,)
            .unwrap_err()
            .code(),
        "store_changed"
    );
    assert_eq!(
        validate_fenced_snapshot(
            &manifest,
            &status,
            &export,
            &serde_json::json!({
                "schema_version": 1,
                "plasmosome.authority-mode": "markdown-shadow",
                "plasmosome.source-commit": "other-source",
            })
            .to_string(),
            &status,
        )
        .unwrap_err()
        .code(),
        "store_changed"
    );

    let mut noncanonical_manifest = manifest.clone();
    noncanonical_manifest.local_generation = format!(" {} ", manifest.local_generation);
    let noncanonical_status = serde_json::json!({
        "schema_version": 1,
        "branch": "main",
        "commit": noncanonical_manifest.local_generation,
    })
    .to_string();
    assert_eq!(
        validate_fenced_snapshot(
            &noncanonical_manifest,
            &noncanonical_status,
            &export,
            &key_values,
            &noncanonical_status,
        )
        .unwrap_err()
        .code(),
        "invalid_store",
        "the state manifest and both vc-status fences require one canonical full commit value"
    );
}

#[test]
fn ordinary_read_plans_are_local_and_write_free() {
    let root = tempdir().unwrap();
    let temporary_repository = root.path().join("temporary-repository");
    let copied_binary = root.path().join("bd");
    fs::create_dir_all(&temporary_repository).unwrap();
    let environment = disposable_environment(root.path());
    let command = |argv: &[&str], cwd: Option<&Path>| CommandSpec {
        program: copied_binary.clone(),
        argv: argv.iter().map(|value| (*value).into()).collect(),
        cwd: cwd.map(Path::to_path_buf),
        environment: environment.clone(),
        redacted_argv_positions: Vec::new(),
    };

    for command in [
        command(&["--version"], None),
        command(
            &["--readonly", "--sandbox", "--json", "vc", "status"],
            Some(&temporary_repository),
        ),
        command(
            &["--readonly", "--sandbox", "export"],
            Some(&temporary_repository),
        ),
        command(
            &["--readonly", "--sandbox", "--json", "kv", "list"],
            Some(&temporary_repository),
        ),
    ] {
        validate_read_command(&command, &temporary_repository, &copied_binary)
            .expect("the exact disposable read protocol is allowed");
    }
    for command in [
        command(&["--sandbox", "export"], Some(&temporary_repository)),
        command(
            &["--readonly", "--sandbox", "ready"],
            Some(&temporary_repository),
        ),
        command(&["--readonly", "--sandbox", "export"], Some(root.path())),
    ] {
        assert_eq!(
            validate_read_command(&command, &temporary_repository, &copied_binary)
                .unwrap_err()
                .code(),
            "invalid_read_command"
        );
    }
    let mut leaked_environment = environment.clone();
    leaked_environment.insert("SSH_AUTH_SOCK".into(), "/private/agent.sock".into());
    let leaked = CommandSpec {
        program: copied_binary.clone(),
        argv: vec!["--version".into()],
        cwd: None,
        environment: leaked_environment,
        redacted_argv_positions: Vec::new(),
    };
    assert_eq!(
        validate_read_command(&leaked, &temporary_repository, &copied_binary)
            .unwrap_err()
            .code(),
        "invalid_read_command"
    );
}

#[test]
fn ordinary_read_locator_plans_are_exact_and_local() {
    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    fs::create_dir_all(&checkout).unwrap();
    let command = |argv: &[&str]| CommandSpec {
        program: PathBuf::from("git"),
        argv: argv.iter().map(|value| (*value).into()).collect(),
        cwd: Some(checkout.clone()),
        environment: BTreeMap::new(),
        redacted_argv_positions: Vec::new(),
    };

    for command in [
        command(&["rev-parse", "--show-toplevel"]),
        command(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
    ] {
        validate_read_locator_command(&command, &checkout)
            .expect("only the two outer local locator forms are admitted");
    }

    for command in [
        command(&["ls-remote", "origin"]),
        command(&["rev-parse", "origin/main^{commit}"]),
        command(&["fetch", "origin"]),
    ] {
        assert_eq!(
            validate_read_locator_command(&command, &checkout)
                .unwrap_err()
                .code(),
            "invalid_read_command"
        );
    }
}

#[test]
fn bootstrap_and_read_validators_are_distinct() {
    let root = tempdir().unwrap();
    let source = root.path().join("source");
    let repository = root.path().join("generation/repository");
    let binary = root.path().join("generation/bd");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&repository).unwrap();
    let command = |program: PathBuf, argv: &[&str], cwd: &Path| CommandSpec {
        program,
        argv: argv.iter().map(|value| (*value).into()).collect(),
        cwd: Some(cwd.to_path_buf()),
        environment: BTreeMap::new(),
        redacted_argv_positions: Vec::new(),
    };

    let bootstrap_write = command(
        binary.clone(),
        &[
            "--sandbox",
            "dolt",
            "commit",
            "-m",
            "bootstrap markdown-shadow aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ],
        &repository,
    );
    validate_bootstrap_command(&bootstrap_write, &source, &repository, &binary)
        .expect("the exact local bootstrap commit is admitted");
    assert_eq!(
        validate_read_command(&bootstrap_write, &repository, &binary)
            .unwrap_err()
            .code(),
        "invalid_read_command"
    );

    for bootstrap_command in [
        command(PathBuf::from("git"), &["init", "--quiet"], &repository),
        command(
            PathBuf::from("git"),
            &["config", "dolt.auto-push", "false"],
            &repository,
        ),
        command(
            binary.clone(),
            &[
                "--sandbox",
                "init",
                "--stealth",
                "--skip-agents",
                "--skip-hooks",
                "--non-interactive",
            ],
            &repository,
        ),
        command(
            binary.clone(),
            &["--sandbox", "import", "/tmp/input.jsonl", "--json"],
            &repository,
        ),
        command(
            binary.clone(),
            &[
                "--sandbox",
                "kv",
                "set",
                "plasmosome.authority-mode",
                "markdown-shadow",
            ],
            &repository,
        ),
        command(binary.clone(), &["--sandbox", "export"], &repository),
        command(
            binary.clone(),
            &["--readonly", "--sandbox", "--json", "vc", "status"],
            &repository,
        ),
        command(
            PathBuf::from("git"),
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                "origin/main^{commit}",
            ],
            &source,
        ),
    ] {
        validate_bootstrap_command(&bootstrap_command, &source, &repository, &binary)
            .expect("an exact bootstrap/source command is admitted");
    }

    let native_projection = command(binary, &["--sandbox", "ready"], &repository);
    assert_eq!(
        validate_bootstrap_command(
            &native_projection,
            &source,
            &repository,
            &root.path().join("generation/bd")
        )
        .unwrap_err()
        .code(),
        "invalid_bootstrap_command"
    );
}

#[test]
fn bootstrap_verifies_artifacts_before_opening_clone_state() {
    let root = tempdir().unwrap();
    let archive = root.path().join("wrong-archive");
    let binary = root.path().join("wrong-bd");
    fs::write(&archive, "wrong archive").unwrap();
    fs::write(&binary, "wrong binary").unwrap();
    let request = BootstrapRequest {
        checkout: root.path().join("not-a-checkout"),
        source_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        source_ref: "origin/main".into(),
        archive,
        binary,
        wrapper: std::env::current_exe().unwrap(),
        host_target: "aarch64-apple-darwin".into(),
    };

    let error = bootstrap(&request).unwrap_err();
    assert_eq!(error.code(), "beads_checksum_mismatch");
    assert!(!request.checkout.exists());
}
