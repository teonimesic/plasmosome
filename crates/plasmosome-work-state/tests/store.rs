use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use plasmosome_work_state::command::{CommandOutput, CommandSpec, RecordingCommandRunner};
use plasmosome_work_state::document::parse_document;
use plasmosome_work_state::freshness::RemoteRelation;
use plasmosome_work_state::pin::PinManifest;
use plasmosome_work_state::shadow::{
    canonical_logical_export, canonical_operational_projection, decode_operational_beads_jsonl,
    initial_operational_metadata, logical_export_digest, operational_projection_digest,
    to_operational_beads_jsonl,
};
use plasmosome_work_state::store::{
    ActivationFault, BootstrapLock, BootstrapRequest, CurrentGeneration, GenerationActivationLock,
    StateManifest, StoreLocation, activate_staged_generation, active_generation, bootstrap,
    current_generation, generation_for_installed_wrapper, host_target, locate_store,
    locator_environment, read_disposable_snapshot, validate_bootstrap_command,
    validate_fenced_snapshot, validate_read_command, validate_read_locator_command,
};
use tempfile::tempdir;

#[cfg(unix)]
use sha2::{Digest, Sha256};

#[cfg(unix)]
fn regular_tree_snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, u32, SystemTime)> {
    use std::os::unix::fs::PermissionsExt;

    fn visit(
        root: &Path,
        path: &Path,
        snapshot: &mut BTreeMap<PathBuf, (Vec<u8>, u32, SystemTime)>,
    ) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(!metadata.file_type().is_symlink());
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        let contents = if metadata.file_type().is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), snapshot);
            }
            Vec::new()
        } else {
            assert!(metadata.file_type().is_file());
            fs::read(path).unwrap()
        };
        snapshot.insert(
            relative,
            (
                contents,
                metadata.permissions().mode(),
                metadata.modified().unwrap(),
            ),
        );
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
}

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
    let result = locate_store(&mut runner, checkout, locator_environment().unwrap())
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
        let error =
            locate_store(&mut runner, &checkout_a, locator_environment().unwrap()).unwrap_err();
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
        let error =
            locate_store(&mut runner, &checkout_a, locator_environment().unwrap()).unwrap_err();
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

#[test]
fn installed_wrapper_reads_its_selected_generation_after_pointer_flip() {
    use sha2::{Digest, Sha256};

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(&location.generations_dir).unwrap();

    let generation = |name: &str, contents: &str| {
        let generation = location.generations_dir.join(name);
        fs::create_dir_all(&generation).unwrap();
        let wrapper = generation.join("plasmosome-work-state");
        fs::write(&wrapper, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut manifest: StateManifest = serde_json::from_str(&valid_manifest()).unwrap();
        manifest.wrapper_sha256 = format!("{:x}", Sha256::digest(contents.as_bytes()));
        fs::write(
            generation.join("state.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        wrapper
    };
    let wrapper_a = generation("generation-a", "selected old wrapper");
    let _wrapper_b = generation("generation-b", "selected new wrapper");
    fs::write(location.state_root.join("current"), "generation-b\n").unwrap();

    assert_eq!(active_generation(&location).unwrap().name, "generation-b");
    let selected = generation_for_installed_wrapper(&location, &wrapper_a)
        .expect("an already selected retained wrapper must bind to its own immutable generation");
    assert_eq!(selected.name, "generation-a");
    assert_eq!(selected.root, location.generations_dir.join("generation-a"));
    assert_eq!(
        selected.manifest.wrapper_sha256,
        format!("{:x}", Sha256::digest(b"selected old wrapper"))
    );
}

#[cfg(unix)]
#[test]
fn installed_wrapper_generation_refuses_unsafe_executable_layouts() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(&location.generations_dir).unwrap();
    let generation = location.generations_dir.join("generation-safe");
    fs::create_dir_all(&generation).unwrap();
    let wrapper = generation.join("plasmosome-work-state");
    fs::write(&wrapper, "verified wrapper").unwrap();
    let mut manifest: StateManifest = serde_json::from_str(&valid_manifest()).unwrap();
    manifest.wrapper_sha256 = format!("{:x}", Sha256::digest(b"verified wrapper"));
    fs::write(
        generation.join("state.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(location.state_root.join("current"), "generation-safe\n").unwrap();

    let outside = root.path().join("outside/plasmosome-work-state");
    fs::create_dir_all(outside.parent().unwrap()).unwrap();
    fs::write(&outside, "verified wrapper").unwrap();
    let wrong_name = generation.join("not-the-wrapper");
    fs::write(&wrong_name, "verified wrapper").unwrap();
    let unsafe_parent = location
        .generations_dir
        .join("not-a-generation/plasmosome-work-state");
    fs::create_dir_all(unsafe_parent.parent().unwrap()).unwrap();
    fs::write(&unsafe_parent, "verified wrapper").unwrap();
    let symlinked = location
        .generations_dir
        .join("generation-link/plasmosome-work-state");
    symlink(
        &generation,
        location.generations_dir.join("generation-link"),
    )
    .unwrap();
    let directory = location
        .generations_dir
        .join("generation-directory/plasmosome-work-state");
    fs::create_dir_all(&directory).unwrap();

    for executable in [
        &outside,
        &wrong_name,
        &unsafe_parent,
        &symlinked,
        &directory,
    ] {
        assert_eq!(
            generation_for_installed_wrapper(&location, executable)
                .expect_err("only one direct regular immutable generation wrapper is accepted")
                .code(),
            "invalid_store",
            "{executable:?}"
        );
    }

    fs::write(&wrapper, "tampered wrapper").unwrap();
    assert_eq!(
        generation_for_installed_wrapper(&location, &wrapper)
            .expect_err("the wrapper hash remains bound to the selected generation")
            .code(),
        "invalid_store"
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

#[cfg(unix)]
#[test]
fn ordinary_reads_refuse_an_unsafe_installed_binary_mode() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let generation_root = root.path().join("generation-safe");
    fs::create_dir_all(&generation_root).unwrap();
    disposable_environment(&generation_root);
    let binary = generation_root.join("bd");
    fs::write(&binary, "checksum-valid fixture binary").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o777)).unwrap();
    let binary_sha256 = format!("{:x}", Sha256::digest(fs::read(&binary).unwrap()));
    let mut manifest: StateManifest = serde_json::from_str(&valid_manifest()).unwrap();
    manifest.beads_binary_sha256 = binary_sha256.clone();
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
        binary_sha256,
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

#[cfg(unix)]
#[test]
fn activation_requires_a_recursively_regular_staged_tree_before_replacing_current() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    let redirected = root.path().join("redirected");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(location.generations_dir.join("generation-old")).unwrap();
    fs::write(location.state_root.join("current"), "generation-old\n").unwrap();
    let staging = location.generations_dir.join(".staging-new");
    fs::create_dir_all(staging.join("repository")).unwrap();
    fs::write(&redirected, "outside staged generation").unwrap();
    symlink(&redirected, staging.join("repository/escaped")).unwrap();

    let error = activate_staged_generation(&location, &staging, "generation-new", None)
        .expect_err("a staged tree with a nested symlink must not be activated");

    assert_eq!(error.code(), "invalid_store");
    assert_eq!(
        fs::read_to_string(location.state_root.join("current")).unwrap(),
        "generation-old\n"
    );
    assert!(staging.exists());
    assert!(!location.generations_dir.join("generation-new").exists());
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

#[test]
fn bootstrap_and_sync_contend_on_one_generation_lock() {
    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let common = root.path().join("common");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&common).unwrap();
    let location = locate(&checkout, &checkout, &common).unwrap();
    fs::create_dir_all(&location.state_root).unwrap();

    let bootstrap = BootstrapLock::acquire(&location).expect("bootstrap holds activation lock");
    assert_eq!(
        GenerationActivationLock::acquire_for_sync(&location)
            .expect_err("sync never waits behind bootstrap")
            .code(),
        "sync_busy"
    );
    drop(bootstrap);
    GenerationActivationLock::acquire_for_sync(&location)
        .expect("sync acquires the released activation lock");
}

#[cfg(unix)]
#[test]
fn bootstrap_lock_syncs_the_common_directory_when_creating_state_root() {
    let root = tempdir().unwrap();
    let state_root = root.path().join("state-root");
    let location = StoreLocation {
        worktree_root: root.path().to_path_buf(),
        common_dir: PathBuf::from("/dev/null"),
        state_root: state_root.clone(),
        generations_dir: state_root.join("generations"),
    };

    let error = BootstrapLock::acquire(&location)
        .expect_err("first creation must durably sync the state-root parent directory");

    assert_eq!(error.code(), "invalid_store");
    assert!(
        state_root.is_dir(),
        "state-root creation happened before parent sync"
    );

    let retry = BootstrapLock::acquire(&location)
        .expect_err("a retry must not bypass the unresolved parent-directory durability barrier");
    assert_eq!(retry.code(), "invalid_store");
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

#[cfg(unix)]
#[test]
fn ordinary_read_uses_and_removes_a_disposable_store_copy() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let generation_root = root.path().join("generation-safe");
    let repository = generation_root.join("repository");
    fs::create_dir_all(repository.join("nested")).unwrap();
    fs::write(
        repository.join("nested/retained"),
        "shared repository contents",
    )
    .unwrap();
    disposable_environment(&generation_root);
    let binary = generation_root.join("bd");
    fs::write(&binary, "verified binary bytes").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let source_commit = "a".repeat(40);
    let local_generation = "d".repeat(40);
    let documents = vec![
        parse_document(
            "docs/intents/001-intent.md",
            "---\nid: 001\ntitle: Intent\nstatus: approved\n---\n",
            &source_commit,
        )
        .unwrap(),
    ];
    let operational = initial_operational_metadata(&documents).unwrap();
    let export = to_operational_beads_jsonl(&documents, &operational).unwrap();
    let operational_documents = decode_operational_beads_jsonl(&export).unwrap();
    let binary_sha256 = format!("{:x}", Sha256::digest(fs::read(&binary).unwrap()));
    let pin = PinManifest::parse(&format!(
        "version = \"1.1.2\"\nrelease = \"https://example.invalid/release\"\nsource_commit = \"{}\"\nlicense = \"MIT\"\nchecksums_url = \"https://example.invalid/checksums\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"aarch64-apple-darwin\"\narchive = \"beads_1.1.2_darwin_arm64.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
        "b".repeat(40),
        "c".repeat(64),
        "e".repeat(64),
        binary_sha256,
    ))
    .unwrap();
    let generation = CurrentGeneration {
        name: "generation-safe".into(),
        root: generation_root.clone(),
        manifest: StateManifest {
            schema_version: 1,
            authority_mode: "markdown-shadow".into(),
            source_commit: source_commit.clone(),
            logical_export_sha256: logical_export_digest(
                &canonical_logical_export(&documents).unwrap(),
            ),
            operational_projection_sha256: operational_projection_digest(
                &canonical_operational_projection(&operational_documents).unwrap(),
            ),
            local_generation: local_generation.clone(),
            host_target: "aarch64-apple-darwin".into(),
            wrapper_sha256: "f".repeat(64),
            beads_binary_sha256: binary_sha256,
            remote_relation: RemoteRelation::Unknown,
            remote_generation: None,
            remote_observed_at: None,
            observed_local_generation: None,
            last_successful_sync_at: None,
            pending_operation_ids: Vec::new(),
        },
    };
    let status = serde_json::json!({
        "schema_version": 1,
        "branch": "main",
        "commit": local_generation,
    })
    .to_string();
    let keys = serde_json::json!({
        "schema_version": 1,
        "plasmosome.authority-mode": "markdown-shadow",
        "plasmosome.source-commit": source_commit,
    })
    .to_string();
    let before = regular_tree_snapshot(&generation_root);
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
        Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
        Ok(CommandOutput::success(status.clone())),
        Ok(CommandOutput::success(export)),
        Ok(CommandOutput::success(keys)),
        Ok(CommandOutput::success(status)),
    ]);

    let snapshot = read_disposable_snapshot(&mut runner, &generation, &pin, "aarch64-apple-darwin")
        .expect("the verified shared generation is read through one disposable copy");

    assert_eq!(snapshot.documents.len(), 1);
    assert_eq!(runner.commands().len(), 6);
    assert_eq!(runner.commands()[0].program, generation_root.join("bd"));
    let temporary_root = runner.commands()[1].program.parent().unwrap().to_path_buf();
    assert_ne!(temporary_root, generation_root);
    assert!(
        runner
            .commands()
            .iter()
            .skip(1)
            .all(|command| !command.program.starts_with(&generation_root)),
        "every command after the shared-runtime verification must use copied files"
    );
    assert!(
        !temporary_root.exists(),
        "the disposable repository, runtime, and copied binary must be removed before returning"
    );
    assert_eq!(regular_tree_snapshot(&generation_root), before);
    assert!(runner.finish().is_ok());
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
    let environment = locator_environment().unwrap();
    let command = |argv: &[&str]| CommandSpec {
        program: PathBuf::from("git"),
        argv: argv.iter().map(|value| (*value).into()).collect(),
        cwd: Some(checkout.clone()),
        environment: environment.clone(),
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
fn ordinary_read_locator_refuses_unsealed_environment_before_dispatch() {
    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    fs::create_dir_all(&checkout).unwrap();
    let expected_environment = BTreeMap::from([
        ("PATH".into(), std::env::var("PATH").unwrap()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
    ]);
    let exact = CommandSpec {
        program: PathBuf::from("git"),
        argv: vec!["rev-parse".into(), "--show-toplevel".into()],
        cwd: Some(checkout.clone()),
        environment: expected_environment.clone(),
        redacted_argv_positions: Vec::new(),
    };
    validate_read_locator_command(&exact, &checkout)
        .expect("the independently constructed ordinary locator environment is exact");

    let mut missing_safety_flag = expected_environment.clone();
    missing_safety_flag.remove("GIT_NO_LAZY_FETCH");
    let mut candidates = vec![missing_safety_flag];
    for (key, value) in [
        ("GITHUB_TOKEN", "sentinel"),
        ("GIT_CONFIG_GLOBAL", "/private/global-gitconfig"),
        ("HTTPS_PROXY", "http://proxy.invalid"),
        ("SSH_AUTH_SOCK", "/private/agent.sock"),
    ] {
        let mut environment = expected_environment.clone();
        environment.insert(key.into(), value.into());
        candidates.push(environment);
    }

    for environment in candidates {
        let mut runner = RecordingCommandRunner::default();
        let error = locate_store(&mut runner, &checkout, environment)
            .expect_err("an unsealed ordinary locator must refuse before Git dispatch");
        assert_eq!(error.code(), "invalid_store_location");
        assert!(
            runner.commands().is_empty(),
            "the rejected ordinary locator environment must not reach the command runner"
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

#[cfg(unix)]
#[derive(Clone, Copy)]
enum BootstrapSnapshotFault {
    StoreChanged,
    SnapshotMismatch,
    InvalidRepository,
    CopiedBinary,
}

#[cfg(unix)]
struct BootstrapSnapshotFixture {
    _root: tempfile::TempDir,
    state_root: PathBuf,
    supplied_calls: PathBuf,
    request: BootstrapRequest,
}

#[cfg(unix)]
fn fixture_digest(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

#[cfg(unix)]
fn fixture_git_environment(root: &Path) -> BTreeMap<String, String> {
    let fixture_root = root
        .parent()
        .expect("fixture checkout must have a containing root");
    let disposable = disposable_environment(fixture_root);
    [
        "PATH",
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "TMPDIR",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_TERMINAL_PROMPT",
        "GIT_NO_LAZY_FETCH",
        "GIT_OPTIONAL_LOCKS",
    ]
    .into_iter()
    .map(|key| {
        (
            key.to_owned(),
            disposable
                .get(key)
                .expect("disposable Git fixture environment must include the key")
                .clone(),
        )
    })
    .collect()
}

#[cfg(unix)]
fn fixture_git(root: &Path, arguments: &[&str]) -> String {
    let fixture_root = root
        .parent()
        .expect("fixture checkout must have a containing root");
    let hooks = fixture_root.join("empty-git-hooks");
    let templates = fixture_root.join("empty-git-templates");
    fs::create_dir_all(&hooks).unwrap();
    fs::create_dir_all(&templates).unwrap();
    let mut command_arguments = vec![
        "-c".to_owned(),
        format!("core.hooksPath={}", hooks.display()),
        "-c".to_owned(),
        format!("init.templateDir={}", templates.display()),
        "-c".to_owned(),
        "user.name=Plasmosome fixture".to_owned(),
        "-c".to_owned(),
        "user.email=fixture@example.invalid".to_owned(),
    ];
    command_arguments.extend(arguments.iter().map(|argument| (*argument).to_owned()));
    let output = std::process::Command::new("git")
        .args(command_arguments)
        .current_dir(root)
        .env_clear()
        .envs(fixture_git_environment(root))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[cfg(unix)]
#[test]
fn fixture_git_is_hermetic_and_does_not_run_hooks() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let sentinel = root.path().join("pre-commit-ran");
    fs::create_dir_all(&checkout).unwrap();
    fs::write(checkout.join("fixture"), "fixture\n").unwrap();
    fixture_git(&checkout, &["init", "--quiet"]);

    let hook = checkout.join(".git/hooks/pre-commit");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(
        &hook,
        format!("#!/bin/sh\nprintf hook > '{}'\n", sentinel.display()),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    fixture_git(
        &checkout,
        &[
            "config",
            "core.hooksPath",
            hook.parent().unwrap().to_str().unwrap(),
        ],
    );
    fixture_git(&checkout, &["add", "."]);
    fixture_git(
        &checkout,
        &[
            "-c",
            "user.name=Plasmosome fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );

    assert!(
        !sentinel.exists(),
        "fixture Git commands must not inherit a repository hook"
    );
}

#[cfg(unix)]
fn fixture_beads_script(fault: BootstrapSnapshotFault, local_generation: &str) -> String {
    let status = match fault {
        BootstrapSnapshotFault::StoreChanged => format!(
            "printf '%s' '{{\"schema_version\":1,\"branch\":\"main\",\"commit\":\"{}\"}}'\n",
            "e".repeat(40)
        ),
        BootstrapSnapshotFault::SnapshotMismatch => format!(
            "if [ -e \"${{0}}.status\" ]; then commit={}; else : > \"${{0}}.status\"; commit={}; fi\nprintf '%s' \"{{\\\"schema_version\\\":1,\\\"branch\\\":\\\"main\\\",\\\"commit\\\":\\\"$commit\\\"}}\"\n",
            "e".repeat(40),
            local_generation,
        ),
        BootstrapSnapshotFault::InvalidRepository | BootstrapSnapshotFault::CopiedBinary => {
            format!(
                "printf '%s' '{{\"schema_version\":1,\"branch\":\"main\",\"commit\":\"{local_generation}\"}}'\n"
            )
        }
    };
    let version = match fault {
        BootstrapSnapshotFault::CopiedBinary => {
            "case \"$0\" in\n  */supplied-bd) printf x >> \"$0.calls\" ;;\n  */generations/generation-old/bd) if [ -e \"$0.seen\" ]; then printf '# corrupt copied binary\\n' >> \"$0\"; else : > \"$0.seen\"; fi ;;\nesac\nprintf 'bd version 1.1.2 (fixture)\\n'\n"
        }
        _ => {
            "case \"$0\" in\n  */supplied-bd) printf x >> \"$0.calls\" ;;\nesac\nprintf 'bd version 1.1.2 (fixture)\\n'\n"
        }
    };
    format!(
        "#!/bin/sh\ncase \"$*\" in\n  \"--version\")\n    {version}    ;;\n  \"--readonly --sandbox --json vc status\")\n    {status}    ;;\n  \"--readonly --sandbox export\")\n    printf ''\n    ;;\n  \"--readonly --sandbox --json kv list\")\n    printf ''\n    ;;\n  *) exit 99 ;;\nesac\n"
    )
}

#[cfg(unix)]
fn bootstrap_snapshot_fixture(fault: BootstrapSnapshotFault) -> BootstrapSnapshotFixture {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let checkout = root.path().join("checkout");
    fs::create_dir_all(checkout.join("docs/intents")).unwrap();
    fs::create_dir_all(checkout.join("tools")).unwrap();
    fs::write(
        checkout.join("docs/intents/001-intent.md"),
        "---\nid: 001\ntitle: Intent\nstatus: approved\n---\n",
    )
    .unwrap();
    fixture_git(&checkout, &["init", "--quiet"]);

    let archive = root.path().join("beads_1.1.2_fixture.tar.gz");
    let supplied_binary = root.path().join("supplied-bd");
    let supplied_calls = PathBuf::from(format!("{}.calls", supplied_binary.display()));
    fs::write(&archive, "fixture archive").unwrap();
    let target = host_target();
    let local_generation = "d".repeat(40);
    let beads_script = fixture_beads_script(fault, &local_generation);
    fs::write(&supplied_binary, &beads_script).unwrap();
    fs::set_permissions(&supplied_binary, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        checkout.join("tools/work-state-beads-1.1.2.toml"),
        format!(
            "version = \"1.1.2\"\nrelease = \"https://example.invalid/release\"\nsource_commit = \"{}\"\nlicense = \"MIT\"\nchecksums_url = \"https://example.invalid/checksums\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"{target}\"\narchive = \"beads_1.1.2_fixture.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
            "a".repeat(40),
            "b".repeat(64),
            fixture_digest(&archive),
            fixture_digest(&supplied_binary),
        ),
    )
    .unwrap();
    fixture_git(&checkout, &["add", "."]);
    fixture_git(
        &checkout,
        &[
            "-c",
            "user.name=Plasmosome fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    let source_commit = fixture_git(&checkout, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let state_root = checkout.join(".git/plasmosome-work-state");
    let generation = state_root.join("generations/generation-old");
    fs::create_dir_all(generation.join("repository")).unwrap();
    fs::write(generation.join("repository/fixture"), "repository fixture").unwrap();
    if matches!(fault, BootstrapSnapshotFault::InvalidRepository) {
        fs::rename(
            generation.join("repository"),
            generation.join("repository-retired"),
        )
        .unwrap();
        fs::write(generation.join("repository"), "not a repository directory").unwrap();
    }
    let wrapper = generation.join("plasmosome-work-state");
    let requested_wrapper = root.path().join("requested-wrapper");
    fs::write(&wrapper, "fixture wrapper").unwrap();
    fs::write(&requested_wrapper, "fixture wrapper").unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&requested_wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let installed_binary = generation.join("bd");
    fs::copy(&supplied_binary, &installed_binary).unwrap();
    fs::set_permissions(&installed_binary, fs::Permissions::from_mode(0o700)).unwrap();
    disposable_environment(&generation);
    let manifest = StateManifest {
        schema_version: 1,
        authority_mode: "markdown-shadow".into(),
        source_commit,
        logical_export_sha256: "b".repeat(64),
        operational_projection_sha256: "c".repeat(64),
        local_generation,
        host_target: target.into(),
        wrapper_sha256: fixture_digest(&wrapper),
        beads_binary_sha256: fixture_digest(&installed_binary),
        remote_relation: RemoteRelation::Unknown,
        remote_generation: None,
        remote_observed_at: None,
        observed_local_generation: None,
        last_successful_sync_at: None,
        pending_operation_ids: Vec::new(),
    };
    fs::write(
        generation.join("state.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(state_root.join("current"), "generation-old\n").unwrap();
    BootstrapSnapshotFixture {
        _root: root,
        state_root,
        supplied_calls,
        request: BootstrapRequest {
            checkout: checkout.clone(),
            source_root: checkout,
            source_ref: "HEAD".into(),
            archive,
            binary: supplied_binary,
            wrapper: requested_wrapper,
            host_target: target.into(),
        },
    }
}

#[cfg(unix)]
fn fixture_generation_names(state_root: &Path) -> Vec<String> {
    let mut names = fs::read_dir(state_root.join("generations"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[cfg(unix)]
fn active_state_snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, u32, SystemTime)> {
    use std::os::unix::fs::PermissionsExt;

    let current = root.join("current");
    let metadata = fs::metadata(&current).unwrap();
    let mut snapshot = regular_tree_snapshot(&root.join("generations"));
    snapshot.insert(
        PathBuf::from("current"),
        (
            fs::read(current).unwrap(),
            metadata.permissions().mode(),
            metadata.modified().unwrap(),
        ),
    );
    snapshot
}

#[cfg(unix)]
fn assert_no_snapshot_recovery(fixture: &BootstrapSnapshotFixture) {
    assert_eq!(
        fs::read_to_string(fixture.state_root.join("current")).unwrap(),
        "generation-old\n"
    );
    assert_eq!(
        fixture_generation_names(&fixture.state_root),
        ["generation-old"]
    );
    assert_eq!(fs::read(&fixture.supplied_calls).unwrap(), b"x");
}

#[cfg(unix)]
#[test]
fn installed_repository_snapshot_failures_are_fatal() {
    for (fault, expected) in [
        (BootstrapSnapshotFault::StoreChanged, "store_changed"),
        (BootstrapSnapshotFault::SnapshotMismatch, "store_changed"),
        (BootstrapSnapshotFault::InvalidRepository, "invalid_store"),
        (
            BootstrapSnapshotFault::CopiedBinary,
            "beads_checksum_mismatch",
        ),
    ] {
        let fixture = bootstrap_snapshot_fixture(fault);
        let before = (!matches!(fault, BootstrapSnapshotFault::CopiedBinary))
            .then(|| active_state_snapshot(&fixture.state_root));
        let error = bootstrap(&fixture.request).unwrap_err();

        assert_eq!(error.code(), expected);
        assert_no_snapshot_recovery(&fixture);
        if let Some(before) = before {
            assert_eq!(active_state_snapshot(&fixture.state_root), before);
        }
    }
}

#[cfg(unix)]
#[test]
fn recovery_snapshot_failure_never_activates_a_generation() {
    let fixture = bootstrap_snapshot_fixture(BootstrapSnapshotFault::InvalidRepository);
    let installed = fixture.state_root.join("generations/generation-old/bd");
    fs::rename(&installed, installed.with_file_name("bd-removed")).unwrap();
    let before = active_state_snapshot(&fixture.state_root);
    let error = bootstrap(&fixture.request).unwrap_err();

    assert_eq!(error.code(), "invalid_store");
    assert_eq!(
        fs::read_to_string(fixture.state_root.join("current")).unwrap(),
        "generation-old\n"
    );
    assert_eq!(
        fixture_generation_names(&fixture.state_root),
        ["generation-old"]
    );
    assert_eq!(fs::read(&fixture.supplied_calls).unwrap(), b"xx");
    assert_eq!(active_state_snapshot(&fixture.state_root), before);
}
