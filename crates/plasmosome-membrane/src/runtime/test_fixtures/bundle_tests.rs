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
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
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
        Self::start_reading(recipe, false).0
    }

    #[allow(unsafe_code)]
    fn start_reading(recipe: &RuntimeRecipe, raw: bool) -> (Self, Option<fs::File>) {
        let cell = recipe.control_path().as_path().parent().unwrap();
        let mode = recipe
            .helper()
            .path()
            .as_path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let mut gate = if mode.starts_with("shim") {
            let path =
                std::ffi::CString::new(cell.join("hello-gate").as_os_str().as_bytes()).unwrap();
            // SAFETY: path is a terminated path in this test's private cell directory.
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            Some(
                fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(cell.join("hello-gate"))
                    .unwrap(),
            )
        } else {
            None
        };
        let control = RuntimeListener::bind(recipe.control_path()).unwrap();
        let data = RuntimeListener::bind(recipe.data_path()).unwrap();
        let prepared = HelperExec::prepare(recipe).unwrap();
        let (transcript, reader) = if raw {
            (None, Some(fs::File::from(prepared.diagnostics)))
        } else {
            (
                Some(DiagnosticTranscript::start(prepared.diagnostics).unwrap()),
                None,
            )
        };
        let child = VmmChild::spawn(prepared.exec).unwrap();
        let run = Self {
            child,
            transcript,
            control,
            data,
        };
        if let Some(gate) = &mut gate {
            let witness = cell.join(format!("before-hello-{}", run.child.pid()));
            let started = Instant::now();
            while !witness.exists() && started.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                witness.exists(),
                "{mode} reached the held before-hello point"
            );
            let marker = cell
                .parent()
                .unwrap()
                .join(format!("markers/started-{}", run.child.pid()));
            assert_eq!(
                fs::read(marker).expect("the helper's start marker exists before its first hello"),
                b"",
                "{mode}"
            );
            gate.write_all(b"G").unwrap();
        }
        (run, reader)
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
    let expected_work = format!("(subpath \"{}\")", bundle.path().join("cell").display());
    let expected_host =
        host_policy::HOST_POLICY.replace(host_policy::WORK_SUBPATH, &expected_work) + "\0";
    assert_eq!(
        fs::read(recipe.host_policy().path().as_path()).unwrap(),
        expected_host.as_bytes()
    );
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
#[allow(unsafe_code)]
fn killing_the_helper_group_closes_the_workers_diagnostic_writer() {
    let bundle = FixtureBundle::build("shim-worker");
    let recipe = bundle.recipe();
    let (mut run, reader) = Run::start_reading(&recipe, true);
    let mut reader = reader.unwrap();
    let association = run.associate(&recipe, Duration::from_secs(2)).unwrap();
    assert_eq!(association.peer, run.child.pid());
    let text = diagnostic_line(&mut reader);
    let worker: i32 = text
        .strip_prefix("worker=")
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(worker > 0 && worker != run.child.pid());
    // SAFETY: getpgid only observes the named pid. No signal is sent to that untrusted number.
    assert_eq!(
        unsafe { libc::getpgid(worker) },
        run.child.pid(),
        "the named worker is alive in the owned helper group"
    );
    let original = worker_identity(worker).expect("the original worker is alive");
    // SAFETY: the unreaped direct child still owns this pid, and the call only signals it.
    assert_eq!(unsafe { libc::kill(run.child.pid(), libc::SIGTERM) }, 0);
    stopped_leader_is_retained(run.child.pid());
    // SAFETY: the terminal leader is retained, not reaped, so this group is still ours.
    assert_eq!(
        unsafe { libc::killpg(run.child.pid(), libc::SIGUSR1) },
        0,
        "the worker group remains after the leader stops"
    );
    assert_eq!(
        diagnostic_line(&mut reader),
        format!("writer={worker}\n"),
        "the live worker itself holds the diagnostic writer after the leader stops"
    );
    run.transcript = Some(DiagnosticTranscript::start(reader.into()).unwrap());
    let transcript = run.finish();
    assert!(transcript.eof, "a worker retained the diagnostic pipe");
    let started = Instant::now();
    while worker_identity(worker) == Some(original)
        && started.elapsed() < Duration::from_millis(250)
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_ne!(
        worker_identity(worker),
        Some(original),
        "the original worker is gone after group teardown"
    );
}

#[allow(unsafe_code)]
fn diagnostic_line(reader: &mut fs::File) -> String {
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut line = Vec::new();
    while line.last() != Some(&b'\n') {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "the worker writes its diagnostic line within the bound"
        );
        let mut poll = libc::pollfd {
            fd: reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll points to one initialized entry for the pipe this test retains.
        let ready = unsafe { libc::poll(&mut poll, 1, left.as_millis().max(1) as i32) };
        if ready < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        assert!(
            ready > 0,
            "the worker writes its diagnostic line within the bound"
        );
        let mut byte = [0];
        assert_eq!(
            reader.read(&mut byte).unwrap(),
            1,
            "the worker holds the diagnostic writer"
        );
        line.push(byte[0]);
        assert!(line.len() < 64, "the worker diagnostic line is bounded");
    }
    String::from_utf8(line).unwrap()
}

#[allow(unsafe_code)]
fn worker_identity(pid: i32) -> Option<u64> {
    let mut record = [0_u64; 7];
    // SAFETY: flavor 17 writes the 56-byte unique-identifier record, whose integer storage is
    // valid for every bit pattern. The call retains no pointer and only observes the pid.
    let size = unsafe { libc::proc_pidinfo(pid, 17, 0, record.as_mut_ptr().cast(), 56) };
    match size {
        56 => Some(record[2]),
        0 => {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            None
        }
        other => panic!("the worker identity record has unexpected size {other}"),
    }
}

#[allow(unsafe_code)]
fn stopped_leader_is_retained(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            Instant::now() < deadline,
            "the leader stops within the existing terminal bound"
        );
        // SAFETY: siginfo_t is C integer and pointer storage; zero initializes a valid value.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: pid is our unreaped direct child; WNOWAIT retains its status and group authority.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as u32,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        assert_eq!(result, 0, "the stopped leader remains waitable");
        if info.si_pid == pid {
            assert_eq!(info.si_code, libc::CLD_KILLED);
            assert_eq!(info.si_status, libc::SIGTERM);
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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
    let mut child = command.spawn().unwrap();
    fake::owned_wait::exit_within(&mut child, fake::owned_wait::SETUP_BOUND)
        .unwrap()
        .expect("the raw recipe child exits within the setup bound")
        .code()
        .unwrap()
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
