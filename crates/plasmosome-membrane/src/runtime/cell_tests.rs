use super::*;
use crate::runtime::closure::{ImageInventory, RuntimeLoads};
use crate::runtime::confinement::PolicyBinding;
use crate::runtime::handshake::Stage;
use crate::runtime::test_fixtures::{FixtureBundle, fake::owned_wait, recipe as parse_recipe};
use std::fs;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;
use std::time::Instant;

fn directory(bundle: &FixtureBundle) -> File {
    File::open(bundle.path().join("cell")).unwrap()
}
fn start(bundle: &FixtureBundle) -> Result<ReadyCell, LaunchFailure> {
    fixture_launch(
        bundle.recipe(),
        directory(bundle).as_fd(),
        Duration::from_secs(10),
        &AtomicBool::new(false),
    )
}

fn fixture_launch(
    recipe: RuntimeRecipe,
    runtime: BorrowedFd<'_>,
    readiness: Duration,
    shutdown: &AtomicBool,
) -> Result<ReadyCell, LaunchFailure> {
    launch_with(recipe, runtime, readiness, shutdown, || Ok(()), |_, _| {})
}

#[test]
fn platform_refusal_precedes_artifacts_directories_and_checkpoints() {
    let value = crate::runtime::test_fixtures::recipe_value("/missing/helper");
    let recipe = parse_recipe(&value);
    let runtime = File::open("/dev/null").unwrap();
    let failure = launch_with(
        recipe,
        runtime.as_fd(),
        Duration::ZERO,
        &AtomicBool::new(false),
        || Err(PlatformRefusal::NoHypervisor),
        |_, _| panic!("platform refusal reached an effect"),
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        LaunchFailure::Platform(PlatformRefusal::NoHypervisor)
    ));
}

#[test]
fn public_launch_keeps_the_real_platform_gate() {
    let runtime = File::open("/dev/null").unwrap();
    struct RestoreReading(Option<HypervisorReading>);
    impl Drop for RestoreReading {
        fn drop(&mut self) {
            HYPERVISOR_READING.set(self.0);
        }
    }
    for (supported, size, errno, expected) in [
        (0, 4, None, Some(PlatformRefusal::NoHypervisor)),
        (2, 4, None, Some(PlatformRefusal::NoHypervisor)),
        (
            1,
            3,
            None,
            Some(PlatformRefusal::Sysctl { errno: libc::EIO }),
        ),
        (
            1,
            8,
            None,
            Some(PlatformRefusal::Sysctl { errno: libc::EIO }),
        ),
        (
            1,
            4,
            Some(libc::EPERM),
            Some(PlatformRefusal::Sysctl { errno: libc::EPERM }),
        ),
        (1, 4, None, None),
    ] {
        let _restore = RestoreReading(HYPERVISOR_READING.replace(Some(HypervisorReading {
            supported,
            size,
            errno,
        })));
        let recipe = parse_recipe(&crate::runtime::test_fixtures::recipe_value(
            "/missing/helper",
        ));
        let failure = launch(
            recipe,
            runtime.as_fd(),
            Duration::ZERO,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        match (expected, failure) {
            (Some(expected), LaunchFailure::Platform(found)) => assert_eq!(found, expected),
            (None, LaunchFailure::Artifact(_)) => {}
            other => panic!("real platform gate changed: {other:?}"),
        }
    }
}

fn clean(report: &TeardownReport) {
    assert_eq!(report.kill, Ok(()));
    assert!(
        report.transcript.eof,
        "diagnostic writer survived teardown: {}",
        String::from_utf8_lossy(&report.transcript.bytes)
    );
    assert_eq!(report.control, RemoveOutcome::Removed);
    assert_eq!(report.data, RemoveOutcome::Removed);
    assert_eq!(report.writable_root, RemoveOutcome::Removed);
}

#[allow(unsafe_code)]
fn gone(bundle: &FixtureBundle, pid: i32) {
    let started = Instant::now();
    loop {
        // SAFETY: signal 0 observes existence without signalling any process. The pid came
        // from the test's owned child; after reap it is diagnostic only.
        let result = unsafe { libc::kill(-pid, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "owned group remains"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut status = 0;
    // SAFETY: waitpid observes only this test's already-reaped child, and status is writable.
    let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    assert_eq!(waited, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    for path in [bundle.recipe().control_path(), bundle.recipe().data_path()] {
        assert!(!path.as_path().exists());
        assert!(UnixStream::connect(path.as_path()).is_err());
    }
    fs::remove_dir(bundle.path().join("cell")).expect("the runtime directory is empty");
}

#[test]
fn k1_ready_retains_the_original_evidence_and_its_limits() {
    let bundle = FixtureBundle::build("shim");
    let mut cell = start(&bundle).unwrap();
    let evidence = cell.evidence();
    assert_eq!(evidence.boot.hex().len(), 64);
    assert_eq!(
        evidence.boot,
        cell.resources.association.as_ref().unwrap().boot
    );
    assert_eq!(
        evidence.guest_policy,
        *bundle.recipe().guest_policy().sha256()
    );
    assert_eq!(
        evidence.host_policy,
        *bundle.recipe().host_policy().sha256()
    );
    assert_eq!(
        evidence.helper_pid,
        cell.resources.child.as_ref().unwrap().pid()
    );
    assert_eq!(evidence.confinement.pid(), evidence.helper_pid);
    assert_eq!(
        evidence.confinement.binding(),
        PolicyBinding::PropertiesObserved
    );
    assert_eq!(
        evidence.closure.loaded_images,
        ImageInventory::NotIndependentlyObserved
    );
    assert_eq!(evidence.closure.runtime_loads, RuntimeLoads::NotAudited);
    assert_eq!(
        evidence.workload_isolation,
        WorkloadObservation::NamespacesAndControlGroupNotObserved
    );
    assert_eq!(evidence.closure.resolved[0].2, ArtifactRole::Library(0));
    assert_eq!(evidence.artifacts.len(), 7);
    assert_eq!(cell.recipe(), &bundle.recipe());
    assert_eq!(cell.state(), Ok(VmmState::Running));
    clean(&cell.teardown());
}

#[test]
fn k2_k12_teardown_and_drop_reap_remove_and_restore_descriptor_count() {
    if std::env::var_os("PLASMOSOME_CELL_DESCRIPTOR_CHILD").is_none() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "runtime::cell::tests::k2_k12_teardown_and_drop_reap_remove_and_restore_descriptor_count", "--nocapture"])
            .env("PLASMOSOME_CELL_DESCRIPTOR_CHILD", "1");
        let mut child = crate::spawn_lock::spawn_guarded(&mut command).unwrap();
        assert!(
            owned_wait::exit_within(&mut child, Duration::from_secs(30))
                .unwrap()
                .expect("isolated descriptor witness finishes")
                .success()
        );
        return;
    }
    for dropped in [false, true] {
        let bundle = FixtureBundle::build("shim");
        let before = super::super::vmm_helper::observe_descriptors()
            .unwrap()
            .len();
        let cell = start(&bundle).unwrap();
        let pid = cell.evidence().helper_pid;
        if dropped {
            drop(cell);
        } else {
            let report = cell.teardown();
            clean(&report);
            assert_eq!(
                report.terminal,
                Ok(VmmState::Signaled {
                    signal: libc::SIGKILL
                })
            );
        }
        gone(&bundle, pid);
        assert_eq!(
            super::super::vmm_helper::observe_descriptors()
                .unwrap()
                .len(),
            before
        );
    }
}

#[test]
fn k3_worker_group_dies_and_the_transcript_reaches_eof() {
    let bundle = FixtureBundle::build("shim-worker");
    let cell = start(&bundle).unwrap();
    let pid = cell.evidence().helper_pid;
    let report = cell.teardown();
    clean(&report);
    assert!(String::from_utf8_lossy(&report.transcript.bytes).contains("worker="));
    gone(&bundle, pid);
}

#[test]
fn k4_unconfined_is_not_ready() {
    let bundle = FixtureBundle::build("shim-unconfined");
    match start(&bundle).unwrap_err() {
        LaunchFailure::Confinement {
            observed: Confinement::Unconfined { .. },
            report,
        } => clean(&report),
        other => panic!("{other:?}"),
    }
}

#[test]
fn k5_dead_helper_refuses_without_spending_the_budget() {
    let bundle = FixtureBundle::build("exit-early");
    let started = Instant::now();
    let failure = fixture_launch(
        bundle.recipe(),
        directory(&bundle).as_fd(),
        Duration::from_secs(30),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(2));
    match failure {
        LaunchFailure::Handshake {
            failure:
                HandshakeFailure::HelperEnded {
                    state: VmmState::Exited { code: 66 },
                    ..
                },
            report,
        } => clean(&report),
        other => panic!("{other:?}"),
    }
}

#[test]
fn k6_silent_helper_spends_only_its_handshake_budget() {
    let bundle = FixtureBundle::build("silent");
    match fixture_launch(
        bundle.recipe(),
        directory(&bundle).as_fd(),
        Duration::from_millis(500),
        &AtomicBool::new(false),
    )
    .unwrap_err()
    {
        LaunchFailure::Handshake {
            failure:
                HandshakeFailure::TimedOut {
                    stage: Stage::NormalAccept,
                },
            report,
        } => clean(&report),
        other => panic!("{other:?}"),
    }
}

#[test]
fn k7_k8_k9_preflight_refusals_create_no_child_or_residue() {
    for case in 0..3 {
        let bundle = FixtureBundle::build("shim");
        let bundle = match case {
            0 => bundle.with_tampered_digest(ArtifactRole::Kernel),
            1 => bundle.with_occupied_control_path(),
            _ => bundle.with_policy_entry_mismatch(),
        };
        let failure = start(&bundle).unwrap_err();
        assert!(match (case, failure) {
            (
                0,
                LaunchFailure::Artifact(ArtifactRefusal {
                    role: ArtifactRole::Kernel,
                    fault:
                        super::super::artifacts::ArtifactFault::Rule(
                            plasmosome_core::artifact::ArtifactFault::Digest { .. },
                        ),
                    ..
                }),
            ) => true,
            (
                1,
                LaunchFailure::Endpoint(EndpointRefusal::Private {
                    refusal:
                        plasmosome_core::private_socket::PrivateSocketError::AddressInUse { .. },
                    ..
                }),
            ) => true,
            (2, LaunchFailure::Initramfs(InitramfsRefusal::GuestPolicyMismatch { .. })) => true,
            (_, other) => panic!("{other:?}"),
        });
        let recipe = bundle.recipe();
        assert!(!recipe.writable_root().as_path().exists());
        assert!(!recipe.data_path().as_path().exists());
        if case == 1 {
            assert_eq!(
                fs::read(recipe.control_path().as_path()).unwrap(),
                b"occupied"
            );
        } else {
            assert!(!recipe.control_path().as_path().exists());
        }
        assert_eq!(
            fs::read_dir(bundle.path().join("markers")).unwrap().count(),
            0
        );
    }
}

#[test]
fn k10_shutdown_cancels_the_handshake_and_releases_everything() {
    let bundle = FixtureBundle::build("silent");
    let shutdown = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            shutdown.store(true, Ordering::Relaxed);
        });
        match fixture_launch(
            bundle.recipe(),
            directory(&bundle).as_fd(),
            Duration::from_secs(10),
            &shutdown,
        )
        .unwrap_err()
        {
            LaunchFailure::Handshake {
                failure: HandshakeFailure::Cancelled { .. },
                report,
            } => clean(&report),
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn k11_a_different_guest_policy_refuses_and_releases_everything() {
    let bundle = FixtureBundle::build("shim-bad-policy");
    match start(&bundle).unwrap_err() {
        LaunchFailure::Handshake {
            failure:
                HandshakeFailure::Refused {
                    refusal: super::super::hello::HelloRefusal::PolicyMismatch { .. },
                    ..
                },
            report,
        } => clean(&report),
        other => panic!("{other:?}"),
    }
}

#[test]
fn replacement_of_the_created_directory_refuses_before_effects() {
    let bundle = FixtureBundle::build("shim");
    let original = directory(&bundle);
    fs::rename(bundle.path().join("cell"), bundle.path().join("original")).unwrap();
    fs::create_dir(bundle.path().join("cell")).unwrap();
    fs::set_permissions(
        bundle.path().join("cell"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert!(matches!(
        fixture_launch(
            bundle.recipe(),
            original.as_fd(),
            Duration::from_secs(10),
            &AtomicBool::new(false)
        ),
        Err(LaunchFailure::Directory(
            ReconfirmFault::DirectoryIdentity { .. }
        ))
    ));
    for name in ["original", "cell", "markers"] {
        assert_eq!(fs::read_dir(bundle.path().join(name)).unwrap().count(), 0);
    }
}

#[test]
fn every_output_parent_must_match_the_created_directory() {
    let bundle = FixtureBundle::build("shim");
    for field in ["writable_root", "control_path", "data_path"] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&bundle.recipe().to_ndjson()).unwrap();
        value[field] = serde_json::json!(bundle.path().join("markers/elsewhere"));
        assert!(matches!(
            fixture_launch(
                parse_recipe(&value),
                directory(&bundle).as_fd(),
                Duration::from_secs(10),
                &AtomicBool::new(false)
            ),
            Err(LaunchFailure::Directory(
                ReconfirmFault::DirectoryIdentity { .. }
            ))
        ));
        assert_eq!(fs::read_dir(bundle.path().join("cell")).unwrap().count(), 0);
    }
}

#[test]
fn a_queued_connection_closes_when_an_attempt_fails_before_accept() {
    let bundle = FixtureBundle::build("silent");
    let recipe = bundle.recipe();
    let mut attempt = Attempt {
        control: Some(RuntimeListener::bind(recipe.control_path()).unwrap()),
        data: Some(RuntimeListener::bind(recipe.data_path()).unwrap()),
        association: None,
        child: None,
        transcript: None,
        root: None,
    };
    let mut queued = UnixStream::connect(recipe.data_path().as_path()).unwrap();
    queued
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    attempt.cleanup();
    let mut byte = [0];
    match queued.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("queued connection stayed open: {other:?}"),
    }
    assert!(!recipe.control_path().as_path().exists());
    assert!(!recipe.data_path().as_path().exists());
}

#[test]
fn a_helper_ended_after_the_handshake_fails_the_final_check() {
    let bundle = FixtureBundle::build("shim");
    let failure = launch_with(
        bundle.recipe(),
        directory(&bundle).as_fd(),
        Duration::from_secs(10),
        &AtomicBool::new(false),
        || Ok(()),
        |stage, resources| {
            if matches!(stage, Checkpoint::Observed) {
                resources.child.as_mut().unwrap().kill().unwrap();
            }
        },
    )
    .unwrap_err();
    match failure {
        LaunchFailure::Reconfirm {
            detail:
                ReconfirmFault::Helper {
                    state:
                        Ok(VmmState::Signaled {
                            signal: libc::SIGKILL,
                        }),
                },
            report,
        } => clean(&report),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_kernel_replaced_after_confinement_refuses_readiness() {
    let bundle = FixtureBundle::build("shim");
    let recipe = bundle.recipe();
    let path = recipe.kernel().path().as_path();
    let saved = bundle.path().join("boot/saved-kernel");
    let failure = launch_with(
        recipe.clone(),
        directory(&bundle).as_fd(),
        Duration::from_secs(10),
        &AtomicBool::new(false),
        || Ok(()),
        |stage, _| {
            if matches!(stage, Checkpoint::Observed) {
                super::super::test_fixtures::tree::set_flags(path, 0);
                fs::rename(path, &saved).unwrap();
                fs::copy(&saved, path).unwrap();
                super::super::test_fixtures::tree::set_flags(path, libc::UF_IMMUTABLE);
            }
        },
    )
    .unwrap_err();
    match failure {
        LaunchFailure::Reconfirm {
            detail:
                ReconfirmFault::Artifact(ArtifactRefusal {
                    role: ArtifactRole::Kernel,
                    ..
                }),
            report,
        } => clean(&report),
        other => panic!("{other:?}"),
    }
}

#[test]
fn each_lane_closed_after_confinement_refuses_readiness() {
    for lane in 0..3 {
        let bundle = FixtureBundle::build("shim");
        let failure = launch_with(
            bundle.recipe(),
            directory(&bundle).as_fd(),
            Duration::from_secs(10),
            &AtomicBool::new(false),
            || Ok(()),
            |stage, resources| {
                if matches!(stage, Checkpoint::Observed) {
                    assert_eq!(
                        resources.child.as_mut().unwrap().state(),
                        Ok(VmmState::Running)
                    );
                    resources
                        .association
                        .as_ref()
                        .unwrap()
                        .close_lane_for_test(lane);
                }
            },
        )
        .unwrap_err();
        match failure {
            LaunchFailure::Reconfirm {
                detail: ReconfirmFault::Stream { .. },
                report,
            } => clean(&report),
            other => panic!("closed lane {lane}: {other:?}"),
        }
    }
}

#[test]
fn an_unlisted_dependency_refuses_before_any_checkpoint_or_resource() {
    let bundle = FixtureBundle::build("shim");
    let mut value: serde_json::Value =
        serde_json::from_slice(&bundle.recipe().to_ndjson()).unwrap();
    value["libraries"] = serde_json::json!([]);
    let failure = launch_with(
        parse_recipe(&value),
        directory(&bundle).as_fd(),
        Duration::from_secs(10),
        &AtomicBool::new(false),
        || Ok(()),
        |_, _| panic!("closure refusal reached an effect"),
    )
    .unwrap_err();
    assert!(matches!(failure, LaunchFailure::Closure(_)));
    for name in ["cell", "markers"] {
        assert_eq!(fs::read_dir(bundle.path().join(name)).unwrap().count(), 0);
    }
}

#[test]
fn a_directory_swapped_after_bind_refuses_before_prepare() {
    let bundle = FixtureBundle::build("shim");
    let dir = directory(&bundle);
    let path = bundle.path().join("cell");
    let held = bundle.path().join("held");
    let failure = launch_with(
        bundle.recipe(),
        dir.as_fd(),
        Duration::from_secs(10),
        &AtomicBool::new(false),
        || Ok(()),
        |stage, _| match stage {
            Checkpoint::Bound => {
                fs::rename(&path, &held).unwrap();
                fs::create_dir(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            Checkpoint::Prepared => {
                fs::remove_dir(&path).unwrap();
                fs::rename(&held, &path).unwrap();
            }
            _ => {}
        },
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        LaunchFailure::Prelaunch(ReconfirmFault::Directory(_))
    ));
    for name in ["cell", "held", "markers"] {
        assert_eq!(fs::read_dir(bundle.path().join(name)).unwrap().count(), 0);
    }
}

#[test]
fn replacement_of_the_copy_immediately_before_spawn_is_refused() {
    let bundle = FixtureBundle::build("shim");
    let recipe = bundle.recipe();
    let failure = launch_with(
        recipe.clone(),
        directory(&bundle).as_fd(),
        Duration::from_secs(10),
        &AtomicBool::new(false),
        || Ok(()),
        |stage, _| {
            if matches!(stage, Checkpoint::Prepared) {
                fs::rename(
                    recipe.writable_root().as_path(),
                    bundle.path().join("cell/original"),
                )
                .unwrap();
                fs::write(recipe.writable_root().as_path(), b"replacement").unwrap();
            }
        },
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        LaunchFailure::Prelaunch(ReconfirmFault::RootIdentity { .. })
    ));
    assert_eq!(
        fs::read(recipe.writable_root().as_path()).unwrap(),
        b"replacement"
    );
    assert!(!recipe.control_path().as_path().exists());
    assert!(!recipe.data_path().as_path().exists());
    assert_eq!(
        fs::read_dir(bundle.path().join("markers")).unwrap().count(),
        0
    );
}

#[test]
fn the_writable_copy_is_reconfirmed_by_identity_and_kind() {
    let bundle = FixtureBundle::build("shim");
    let recipe = bundle.recipe();
    let a = verify_recipe_artifacts(&recipe).unwrap();
    let dir = PrivateDirectory::open(&recipe.writable_root().parent().unwrap()).unwrap();
    let root = WritableRoot::create(recipe.writable_root(), &dir, &a.root_image).unwrap();
    reconfirm_root(&root).unwrap();
    let path = root.path().as_path();
    let saved = bundle.path().join("cell/saved");
    fs::rename(path, &saved).unwrap();
    assert!(matches!(
        reconfirm_root(&root),
        Err(ReconfirmFault::Io {
            errno: libc::ENOENT
        })
    ));
    fs::write(path, b"replacement").unwrap();
    assert!(matches!(
        reconfirm_root(&root),
        Err(ReconfirmFault::RootIdentity { .. })
    ));
    fs::remove_file(path).unwrap();
    symlink(&saved, path).unwrap();
    assert_eq!(reconfirm_root(&root), Err(ReconfirmFault::RootNotRegular));
    assert_eq!(root.remove(), RemoveOutcome::LeftReplacement);
}
