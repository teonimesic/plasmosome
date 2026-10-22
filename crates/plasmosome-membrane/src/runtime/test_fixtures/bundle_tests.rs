use super::*;
use crate::readiness::ProbeBudget;
use crate::runtime::artifacts::{ArtifactFault, verify_recipe_artifacts};
use crate::runtime::closure::audit_closure;
use crate::runtime::confinement::{Confinement, REQUIRED_DENIALS, observe};
use crate::runtime::endpoints::{DiagnosticTranscript, RuntimeListener, Transcript};
use crate::runtime::handshake::{Association, HandshakeFailure, Liveness, Stage, handshake};
use crate::runtime::hello::HelloRefusal;
use crate::runtime::initramfs::verify_initramfs;
use crate::runtime::launch::HelperExec;
use crate::runtime::test_fixtures::FixtureBundle;
use crate::vmm::{VmmChild, VmmState};
use std::os::macos::fs::MetadataExt as DarwinMetadataExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

struct Run {
    child: VmmChild,
    transcript: Option<DiagnosticTranscript>,
    control: RuntimeListener,
    data: RuntimeListener,
}

impl Run {
    fn start(recipe: &RuntimeRecipe) -> Self {
        let control = RuntimeListener::bind(recipe.control_path()).unwrap();
        let data = RuntimeListener::bind(recipe.data_path()).unwrap();
        let prepared = HelperExec::prepare(recipe).unwrap();
        let transcript = Some(DiagnosticTranscript::start(prepared.diagnostics).unwrap());
        let child = VmmChild::spawn(prepared.exec).unwrap();
        Self {
            child,
            transcript,
            control,
            data,
        }
    }

    fn associate(
        &mut self,
        recipe: &RuntimeRecipe,
        allowance: Duration,
    ) -> Result<Association, HandshakeFailure> {
        let shutdown = AtomicBool::new(false);
        handshake(
            &self.control,
            &self.data,
            self.child.pid(),
            recipe.guest_policy().sha256(),
            &ProbeBudget::new(allowance, &shutdown),
            || match self.child.state() {
                Ok(VmmState::Running) => Liveness::Running,
                Ok(state) => Liveness::Ended(state),
                Err(error) => Liveness::Unknown(error),
            },
        )
    }

    fn finish(&mut self) -> Transcript {
        self.child.kill().unwrap();
        self.transcript
            .take()
            .unwrap()
            .finish(Duration::from_millis(250))
    }
}

#[test]
fn bundle_passes_the_real_artifact_archive_and_listed_library_checks() {
    assert_eq!(
        Digest::of(&[host_policy::HOST_POLICY.as_bytes(), b"\0"].concat()).hex(),
        host_policy::HOST_POLICY_SHA256
    );
    let bundle = FixtureBundle::build("shim");
    let recipe = bundle.recipe();
    let verified = verify_recipe_artifacts(&recipe).unwrap();
    let binding = verify_initramfs(&verified.initramfs, &verified.guest_policy).unwrap();
    assert_eq!(binding.policy_digest, *recipe.guest_policy().sha256());
    let closure = audit_closure(&verified.helper, &verified.libraries).unwrap();
    assert_eq!(
        closure.resolved,
        vec![(
            ArtifactRole::Helper,
            "@executable_path/../lib/libplasmosome-fake.dylib".to_owned(),
            ArtifactRole::Library(0)
        )]
    );
    for path in &bundle.files {
        let metadata = fs::metadata(path).unwrap();
        let mode = if path == recipe.helper().path().as_path() {
            0o555
        } else {
            0o444
        };
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            mode,
            "{}",
            path.display()
        );
        assert_ne!(
            metadata.st_flags() & libc::UF_IMMUTABLE,
            0,
            "{}",
            path.display()
        );
    }
    assert_eq!(verified.root_image.size(), 1 << 20);
    assert_eq!(recipe.memory_mib(), 64);
    assert_eq!(recipe.vcpus(), 1);
    assert_eq!(
        fs::metadata(bundle.path().join("cell")).unwrap().mode() & 0o777,
        0o700
    );
    assert!(
        bundle
            .path()
            .starts_with(fs::canonicalize(std::env::temp_dir()).unwrap())
    );
}

#[test]
fn shim_uses_both_lanes_one_fresh_boot_and_the_real_host_policy() {
    let mut boots = Vec::new();
    for _ in 0..2 {
        let bundle = FixtureBundle::build("shim");
        let recipe = bundle.recipe();
        let mut run = Run::start(&recipe);
        let association = run.associate(&recipe, Duration::from_secs(2)).unwrap();
        assert_eq!(association.policy, *recipe.guest_policy().sha256());
        assert_eq!(association.peer, run.child.pid());
        boots.push(association.boot);
        let observed = observe(run.child.pid());
        let Confinement::Confined(evidence) = observed else {
            panic!("{observed:?}")
        };
        assert_eq!(evidence.pid(), run.child.pid());
        assert_eq!(evidence.denied(), REQUIRED_DENIALS);
        assert_eq!(run.child.state().unwrap(), VmmState::Running);
        assert!(run.finish().eof);
    }
    assert_ne!(boots[0], boots[1]);
}

#[test]
fn unconfined_shim_speaks_the_protocol_but_the_kernel_reports_no_policy() {
    let bundle = FixtureBundle::build("shim-unconfined");
    let recipe = bundle.recipe();
    let mut run = Run::start(&recipe);
    let association = run.associate(&recipe, Duration::from_secs(2)).unwrap();
    assert_eq!(association.policy, *recipe.guest_policy().sha256());
    assert_eq!(
        observe(run.child.pid()),
        Confinement::Unconfined {
            sandboxed: false,
            allowed: REQUIRED_DENIALS.to_vec(),
        }
    );
    assert!(run.finish().eof);
}

#[test]
fn exit_early_records_its_pid_and_the_handshake_observes_exit_66() {
    let bundle = FixtureBundle::build("exit-early");
    assert_eq!(
        fs::read_dir(bundle.path().join("markers")).unwrap().count(),
        0
    );
    let recipe = bundle.recipe();
    let mut run = Run::start(&recipe);
    assert_eq!(
        run.child.wait_terminal(Duration::from_secs(2)).unwrap(),
        VmmState::Exited { code: 66 }
    );
    let marker = bundle
        .path()
        .join(format!("markers/started-{}", run.child.pid()));
    assert_eq!(fs::read(marker).unwrap(), b"");
    assert_eq!(
        fs::read_dir(bundle.path().join("markers")).unwrap().count(),
        1
    );
    assert_eq!(
        run.associate(&recipe, Duration::from_secs(2)).unwrap_err(),
        HandshakeFailure::HelperEnded {
            stage: Stage::NormalAccept,
            state: VmmState::Exited { code: 66 }
        }
    );
    assert!(run.finish().eof);
}

#[test]
fn silent_reads_the_recipe_but_never_connects() {
    let bundle = FixtureBundle::build("silent");
    let recipe = bundle.recipe();
    let mut run = Run::start(&recipe);
    let marker = bundle
        .path()
        .join(format!("markers/started-{}", run.child.pid()));
    let started = Instant::now();
    while !marker.exists() && started.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(marker.exists());
    assert_eq!(
        run.associate(&recipe, Duration::from_millis(300))
            .unwrap_err(),
        HandshakeFailure::TimedOut {
            stage: Stage::NormalAccept
        }
    );
    assert_eq!(run.child.state().unwrap(), VmmState::Running);
    assert!(run.finish().eof);
}

#[test]
fn bad_policy_is_refused_with_the_expected_and_changed_digests() {
    let bundle = FixtureBundle::build("shim-bad-policy");
    let recipe = bundle.recipe();
    let mut run = Run::start(&recipe);
    let expected = *recipe.guest_policy().sha256();
    let mut changed = expected.hex();
    changed.replace_range(..1, if changed.starts_with('0') { "1" } else { "0" });
    assert_eq!(
        run.associate(&recipe, Duration::from_secs(2)).unwrap_err(),
        HandshakeFailure::Refused {
            stage: Stage::NormalReply,
            refusal: HelloRefusal::PolicyMismatch {
                expected,
                found: Digest::parse_hex(&changed).unwrap()
            }
        }
    );
    assert!(run.finish().eof);
}

#[test]
fn killing_the_helper_group_closes_the_workers_diagnostic_writer() {
    let bundle = FixtureBundle::build("shim-worker");
    let recipe = bundle.recipe();
    let mut run = Run::start(&recipe);
    let association = run.associate(&recipe, Duration::from_secs(2)).unwrap();
    assert_eq!(association.peer, run.child.pid());
    let transcript = run.finish();
    assert!(transcript.eof, "a worker retained the diagnostic pipe");
    let text = std::str::from_utf8(&transcript.bytes).unwrap();
    let worker: i32 = text.strip_prefix("worker=").unwrap().trim().parse().unwrap();
    assert!(worker > 0 && worker != run.child.pid());
}

#[test]
fn dropping_a_bundle_removes_even_the_immutable_files() {
    let bundle = FixtureBundle::build("shim");
    let path = bundle.path().to_owned();
    let files = bundle.files.clone();
    drop(bundle);
    let removed = !path.exists();
    if !removed {
        for file in files {
            tree::set_flags(&file, 0);
        }
        fs::remove_dir_all(&path).unwrap();
    }
    assert!(removed, "Drop left the bundle behind");
}

#[test]
fn every_tampered_pin_is_refused_before_a_helper_starts() {
    let mut bundle = FixtureBundle::build("shim");
    let original = bundle.value.clone();
    for role in [
        ArtifactRole::Kernel,
        ArtifactRole::Initramfs,
        ArtifactRole::RootImage,
        ArtifactRole::Helper,
        ArtifactRole::Library(0),
        ArtifactRole::HostPolicy,
        ArtifactRole::GuestPolicy,
    ] {
        bundle.value = original.clone();
        bundle = bundle.with_tampered_digest(role);
        let refusal = verify_recipe_artifacts(&bundle.recipe()).unwrap_err();
        assert_eq!(refusal.role, role);
        assert!(matches!(
            refusal.fault,
            ArtifactFault::Rule(plasmosome_core::artifact::ArtifactFault::Digest { .. })
        ));
        assert_eq!(
            fs::read_dir(bundle.path().join("markers")).unwrap().count(),
            0
        );
    }
}

#[test]
fn an_archive_with_another_policy_passes_artifacts_but_refuses_the_binding() {
    let bundle = FixtureBundle::build("shim").with_policy_entry_mismatch();
    let verified = verify_recipe_artifacts(&bundle.recipe()).unwrap();
    assert!(matches!(
        verify_initramfs(&verified.initramfs, &verified.guest_policy),
        Err(crate::runtime::initramfs::InitramfsRefusal::GuestPolicyMismatch { .. })
    ));
}

#[test]
fn an_occupied_endpoint_is_preserved_without_starting_a_helper() {
    let bundle = FixtureBundle::build("shim").with_occupied_control_path();
    let recipe = bundle.recipe();
    assert!(RuntimeListener::bind(recipe.control_path()).is_err());
    assert_eq!(
        fs::read(recipe.control_path().as_path()).unwrap(),
        b"occupied"
    );
    assert_eq!(
        fs::read_dir(bundle.path().join("markers")).unwrap().count(),
        0
    );
}

#[allow(unsafe_code)]
fn raw_recipe(bundle: &FixtureBundle, bytes: &[u8]) -> i32 {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let path = bundle.path().join("cell/recipe");
    fs::write(&path, bytes).unwrap();
    let mut command = Command::new(bundle.recipe().helper().path().as_path());
    command
        .arg0("exit-early")
        .env_clear()
        .stdin(fs::File::open(path).unwrap())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: stdin is the command's open recipe file in the child. This closure only duplicates
    // it onto fd 3 and returns an OS error on failure; it allocates nothing and takes no lock.
    unsafe {
        command.pre_exec(|| {
            if libc::dup2(0, 3) == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command.status().unwrap().code().unwrap()
}

#[test]
fn the_fake_reads_to_eof_and_refuses_one_byte_past_its_recipe_cap() {
    for (length, code) in [(1 << 20, 66), ((1 << 20) + 1, 65)] {
        let bundle = FixtureBundle::build("exit-early");
        let mut bytes = bundle.recipe().to_ndjson();
        bytes.resize(length, b' ');
        assert_eq!(raw_recipe(&bundle, &bytes), code);
        assert_eq!(
            fs::read_dir(bundle.path().join("markers")).unwrap().count(),
            usize::from(code == 66)
        );
    }
}
