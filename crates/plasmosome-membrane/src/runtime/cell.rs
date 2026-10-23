use super::RemoveOutcome;
use super::artifacts::{ArtifactRefusal, FileIdentity, VerifiedArtifacts, verify_recipe_artifacts};
use super::cell_directory::{CellDirectoryRefusal, PrivateDirectory, WritableRoot};
use super::closure::{ClosureEvidence, ClosureRefusal, audit_closure};
use super::confinement::{Confinement, ConfinementEvidence, observe};
use super::digest::Digest;
use super::endpoints::{DiagnosticTranscript, EndpointRefusal, RuntimeListener, Transcript};
use super::handshake::{Association, HandshakeFailure, Liveness, handshake};
use super::hello::BootToken;
use super::initramfs::{InitramfsBinding, InitramfsRefusal, verify_initramfs};
use super::launch::{HelperExec, LaunchRefusal};
use super::path::RecipePath;
use super::recipe::{ArtifactRole, RuntimeRecipe};
use crate::readiness::ProbeBudget;
use crate::vmm::{SpawnError, SupervisionError, VmmChild, VmmState};
use std::fs::{File, symlink_metadata};
use std::os::fd::BorrowedFd;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// A handshake and host-property milestone, not proof of a booted hardware guest, the exact
/// installed host policy, loaded images, runtime loads, workload namespaces or control group.
/// The closure and confinement records retain their own explicit limits.
#[derive(Debug)]
pub struct LaunchEvidence {
    pub helper_pid: libc::pid_t,
    pub boot: BootToken,
    pub guest_policy: Digest,
    pub host_policy: Digest,
    pub artifacts: Vec<(ArtifactRole, RecipePath, Digest, FileIdentity)>,
    pub initramfs: InitramfsBinding,
    pub closure: ClosureEvidence,
    pub confinement: ConfinementEvidence,
    pub os_build: String,
    pub workload_isolation: WorkloadObservation,
}

/// What this host composition has observed of the guest's workload isolation.
#[derive(Debug, PartialEq, Eq)]
pub enum WorkloadObservation {
    /// Neither the workload namespaces nor its control group were observed.
    NamespacesAndControlGroupNotObserved,
}

/// One original helper, its streams and resources. Keep it until destructive teardown.
/// Drop uses the same teardown as a fallback; it is not spec 023's disk-preserving clean stop.
pub struct ReadyCell {
    recipe: RuntimeRecipe,
    _artifacts: VerifiedArtifacts,
    _directories: Vec<PrivateDirectory>,
    resources: Attempt,
    evidence: LaunchEvidence,
}

impl std::fmt::Debug for ReadyCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadyCell")
            .field("evidence", &self.evidence)
            .finish_non_exhaustive()
    }
}

impl ReadyCell {
    /// The evidence observed before this value was returned, with its stated limits.
    pub fn evidence(&self) -> &LaunchEvidence {
        &self.evidence
    }
    /// The deployment input retained by this original supervisor, never recovery authority.
    pub fn recipe(&self) -> &RuntimeRecipe {
        &self.recipe
    }
    /// Observes the exact owned child through its exclusive reaper.
    pub fn state(&mut self) -> Result<VmmState, SupervisionError> {
        self.resources
            .child
            .as_mut()
            .expect("ready cell owns its child")
            .state()
    }
    /// Closes the streams, kills and reaps the managed group, drains diagnostics and removes
    /// both listeners and the writable copy by identity. Attempts every step despite failures.
    /// This destroys the disk; never use it for spec 023's disk-preserving signal stop.
    pub fn teardown(mut self) -> TeardownReport {
        self.resources.cleanup().expect("ready cell owns its child")
    }
}

impl Drop for ReadyCell {
    fn drop(&mut self) {
        self.resources.cleanup();
    }
}

/// Results of every destructive teardown step. Terminal status alone does not name a helper
/// refusal: inspect the drained transcript, especially for exit 70, also used by spawn panic.
#[derive(Debug)]
pub struct TeardownReport {
    pub kill: Result<(), SupervisionError>,
    pub terminal: Result<VmmState, SupervisionError>,
    pub transcript: Transcript,
    pub control: RemoveOutcome,
    pub data: RemoveOutcome,
    pub writable_root: RemoveOutcome,
}

/// Why this host cannot attempt the macOS launch.
#[derive(Debug, PartialEq, Eq)]
pub enum PlatformRefusal {
    UnsupportedHost,
    NoHypervisor,
    Sysctl { errno: i32 },
}

/// A path identity or final liveness check that could not be reconfirmed.
#[derive(Debug, PartialEq, Eq)]
pub enum ReconfirmFault {
    Io {
        errno: i32,
    },
    DirectoryIdentity {
        path: RecipePath,
        expected: FileIdentity,
        found: FileIdentity,
    },
    RootIdentity {
        expected: FileIdentity,
        found: FileIdentity,
    },
    RootNotRegular,
    Artifact(ArtifactRefusal),
    Directory(CellDirectoryRefusal),
    Helper {
        state: Result<VmmState, SupervisionError>,
    },
    Stream {
        errno: i32,
    },
}

/// The stage that refused launch. Pre-spawn failures have no report and release partial
/// resources on return. Every post-spawn refusal includes all destructive teardown results.
#[derive(Debug)]
pub enum LaunchFailure {
    Platform(PlatformRefusal),
    Artifact(ArtifactRefusal),
    Initramfs(InitramfsRefusal),
    Closure(ClosureRefusal),
    CellDirectory(CellDirectoryRefusal),
    Directory(ReconfirmFault),
    Endpoint(EndpointRefusal),
    Prepare(LaunchRefusal),
    Diagnostic {
        errno: i32,
    },
    Prelaunch(ReconfirmFault),
    Spawn(SpawnError),
    Handshake {
        failure: HandshakeFailure,
        report: Box<TeardownReport>,
    },
    Confinement {
        observed: Confinement,
        report: Box<TeardownReport>,
    },
    Reconfirm {
        detail: ReconfirmFault,
        report: Box<TeardownReport>,
    },
}

#[derive(Default)]
struct Attempt {
    association: Option<Association>,
    child: Option<VmmChild>,
    transcript: Option<DiagnosticTranscript>,
    control: Option<RuntimeListener>,
    data: Option<RuntimeListener>,
    root: Option<WritableRoot>,
}

impl Attempt {
    fn cleanup(&mut self) -> Option<TeardownReport> {
        drop(self.association.take());
        let status = self
            .child
            .as_mut()
            .map(|child| (child.kill(), child.state()));
        drop(self.child.take());
        let transcript = self
            .transcript
            .take()
            .map(|t| t.finish(Duration::from_secs(2)))
            .unwrap_or_default();
        let control = self
            .control
            .take()
            .map(RuntimeListener::close)
            .unwrap_or(RemoveOutcome::AlreadyGone);
        let data = self
            .data
            .take()
            .map(RuntimeListener::close)
            .unwrap_or(RemoveOutcome::AlreadyGone);
        let writable_root = self
            .root
            .take()
            .map(WritableRoot::remove)
            .unwrap_or(RemoveOutcome::AlreadyGone);
        status.map(|(kill, terminal)| TeardownReport {
            kill,
            terminal,
            transcript,
            control,
            data,
            writable_root,
        })
    }

    fn report(&mut self) -> Box<TeardownReport> {
        Box::new(
            self.cleanup()
                .expect("spawn succeeded before a reported refusal"),
        )
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Verifies deployment input, copies the disk, binds private listeners and launches one owned
/// helper. Returns only after both hellos, data association, host confinement, identity and
/// final child/stream checks. Evidence explicitly records the remaining unobserved properties.
/// Pass the runtime descriptor returned by core's CellDir::create_runtime_dir; every output
/// parent must be that directory. No output is created before its identity matches.
/// The caller must keep SIGCHLD waitable and must never reap this child itself.
pub fn launch(
    recipe: RuntimeRecipe,
    runtime_dir: BorrowedFd<'_>,
    readiness: Duration,
    shutdown: &AtomicBool,
) -> Result<ReadyCell, LaunchFailure> {
    launch_with(recipe, runtime_dir, readiness, shutdown, |_, _| {})
}

#[derive(Clone, Copy)]
enum Checkpoint {
    Bound,
    Prepared,
    Observed,
}

fn launch_with(
    recipe: RuntimeRecipe,
    runtime_dir: BorrowedFd<'_>,
    readiness: Duration,
    shutdown: &AtomicBool,
    mut checkpoint: impl FnMut(Checkpoint, &mut Attempt),
) -> Result<ReadyCell, LaunchFailure> {
    platform().map_err(LaunchFailure::Platform)?;
    let artifacts = verify_recipe_artifacts(&recipe).map_err(LaunchFailure::Artifact)?;
    let initramfs = verify_initramfs(&artifacts.initramfs, &artifacts.guest_policy)
        .map_err(LaunchFailure::Initramfs)?;
    artifacts
        .initramfs
        .reconfirm()
        .map_err(LaunchFailure::Artifact)?;
    let closure =
        audit_closure(&artifacts.helper, &artifacts.libraries).map_err(LaunchFailure::Closure)?;
    let directories = open_directories(&recipe, runtime_dir)?;
    let mut resources = Attempt {
        root: Some(
            WritableRoot::create(
                recipe.writable_root(),
                &directories[0],
                &artifacts.root_image,
            )
            .map_err(LaunchFailure::CellDirectory)?,
        ),
        association: None,
        child: None,
        transcript: None,
        control: None,
        data: None,
    };
    resources.control =
        Some(RuntimeListener::bind(recipe.control_path()).map_err(LaunchFailure::Endpoint)?);
    resources.data =
        Some(RuntimeListener::bind(recipe.data_path()).map_err(LaunchFailure::Endpoint)?);
    checkpoint(Checkpoint::Bound, &mut resources);
    reconfirm_directories(&directories).map_err(LaunchFailure::Prelaunch)?;
    let prepared = HelperExec::prepare(&recipe).map_err(LaunchFailure::Prepare)?;
    resources.transcript = Some(DiagnosticTranscript::start(prepared.diagnostics).map_err(
        |error| LaunchFailure::Diagnostic {
            errno: errno(&error),
        },
    )?);
    checkpoint(Checkpoint::Prepared, &mut resources);
    reconfirm(&artifacts, &directories, resources.root.as_ref().unwrap())
        .map_err(LaunchFailure::Prelaunch)?;
    resources.child = Some(VmmChild::spawn(prepared.exec).map_err(LaunchFailure::Spawn)?);
    let child = resources.child.as_mut().unwrap();
    let pid = child.pid();
    let budget = ProbeBudget::new(readiness, shutdown);
    resources.association = Some(
        handshake(
            resources.control.as_ref().unwrap(),
            resources.data.as_ref().unwrap(),
            pid,
            artifacts.guest_policy.digest(),
            &budget,
            || match child.state() {
                Ok(VmmState::Running) => Liveness::Running,
                Ok(state) => Liveness::Ended(state),
                Err(error) => Liveness::Unknown(error),
            },
        )
        .map_err(|failure| LaunchFailure::Handshake {
            failure,
            report: resources.report(),
        })?,
    );
    let confinement = match observe(pid) {
        Confinement::Confined(evidence) => evidence,
        observed => {
            return Err(LaunchFailure::Confinement {
                observed,
                report: resources.report(),
            });
        }
    };
    checkpoint(Checkpoint::Observed, &mut resources);
    reconfirm(&artifacts, &directories, resources.root.as_ref().unwrap())
        .and_then(|()| final_check(&mut resources))
        .map_err(|detail| LaunchFailure::Reconfirm {
            detail,
            report: resources.report(),
        })?;
    let evidence = LaunchEvidence {
        helper_pid: pid,
        boot: resources.association.as_ref().unwrap().boot,
        guest_policy: *artifacts.guest_policy.digest(),
        host_policy: *artifacts.host_policy.digest(),
        artifacts: artifact_list(&artifacts)
            .into_iter()
            .map(|a| (a.role(), a.path().clone(), *a.digest(), a.identity()))
            .collect(),
        initramfs,
        os_build: closure.os_build.clone(),
        closure,
        confinement,
        workload_isolation: WorkloadObservation::NamespacesAndControlGroupNotObserved,
    };
    Ok(ReadyCell {
        recipe,
        _artifacts: artifacts,
        _directories: directories,
        resources,
        evidence,
    })
}

fn artifact_list(a: &VerifiedArtifacts) -> Vec<&super::artifacts::VerifiedArtifact> {
    [&a.kernel, &a.initramfs, &a.root_image, &a.helper]
        .into_iter()
        .chain(a.libraries.iter())
        .chain([&a.host_policy, &a.guest_policy])
        .collect()
}

fn open_directories(
    recipe: &RuntimeRecipe,
    runtime: BorrowedFd<'_>,
) -> Result<Vec<PrivateDirectory>, LaunchFailure> {
    let file = File::from(
        runtime
            .try_clone_to_owned()
            .map_err(|e| LaunchFailure::Directory(ReconfirmFault::Io { errno: errno(&e) }))?,
    );
    let expected = identity(
        &file
            .metadata()
            .map_err(|e| LaunchFailure::Directory(ReconfirmFault::Io { errno: errno(&e) }))?,
    );
    let mut directories = Vec::new();
    for path in [
        recipe.writable_root(),
        recipe.control_path(),
        recipe.data_path(),
    ] {
        let parent = path.parent().expect("recipe output has an absolute parent");
        if directories
            .iter()
            .any(|dir: &PrivateDirectory| dir.path() == &parent)
        {
            continue;
        }
        let dir = PrivateDirectory::open(&parent).map_err(LaunchFailure::CellDirectory)?;
        let found = dir.identity();
        if found != expected {
            return Err(LaunchFailure::Directory(
                ReconfirmFault::DirectoryIdentity {
                    path: parent,
                    expected,
                    found,
                },
            ));
        }
        directories.push(dir);
    }
    Ok(directories)
}

fn identity(metadata: &std::fs::Metadata) -> FileIdentity {
    FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

fn reconfirm_directories(directories: &[PrivateDirectory]) -> Result<(), ReconfirmFault> {
    for dir in directories {
        dir.reconfirm().map_err(ReconfirmFault::Directory)?;
    }
    Ok(())
}

fn reconfirm(
    a: &VerifiedArtifacts,
    directories: &[PrivateDirectory],
    root: &WritableRoot,
) -> Result<(), ReconfirmFault> {
    for artifact in artifact_list(a) {
        artifact.reconfirm().map_err(ReconfirmFault::Artifact)?;
    }
    reconfirm_directories(directories)?;
    reconfirm_root(root)
}

fn reconfirm_root(root: &WritableRoot) -> Result<(), ReconfirmFault> {
    let metadata = symlink_metadata(root.path().as_path())
        .map_err(|e| ReconfirmFault::Io { errno: errno(&e) })?;
    if !metadata.is_file() {
        return Err(ReconfirmFault::RootNotRegular);
    }
    let found = identity(&metadata);
    let expected = root.identity();
    if found != expected {
        return Err(ReconfirmFault::RootIdentity { expected, found });
    }
    Ok(())
}

fn final_check(resources: &mut Attempt) -> Result<(), ReconfirmFault> {
    let state = resources.child.as_mut().unwrap().state();
    if state != Ok(VmmState::Running) {
        return Err(ReconfirmFault::Helper { state });
    }
    resources
        .association
        .as_ref()
        .unwrap()
        .check_open()
        .map_err(|e| ReconfirmFault::Stream { errno: errno(&e) })?;
    Ok(())
}

fn errno(error: &std::io::Error) -> i32 {
    error.raw_os_error().unwrap_or(libc::EIO)
}

#[cfg(not(target_os = "macos"))]
fn platform() -> Result<(), PlatformRefusal> {
    Err(PlatformRefusal::UnsupportedHost)
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn platform() -> Result<(), PlatformRefusal> {
    let mut supported: i32 = 0;
    let mut size = std::mem::size_of_val(&supported);
    // SAFETY: sysctlbyname writes at most size bytes into the live i32 and retains no pointer;
    // the name is terminated and the null new-value pointer requests observation only.
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.hv_support".as_ptr(),
            (&raw mut supported).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(PlatformRefusal::Sysctl {
            errno: errno(&std::io::Error::last_os_error()),
        });
    }
    if size != std::mem::size_of_val(&supported) {
        return Err(PlatformRefusal::Sysctl { errno: libc::EIO });
    }
    if supported != 1 {
        return Err(PlatformRefusal::NoHypervisor);
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
#[path = "cell_tests.rs"]
mod tests;

#[cfg(all(test, not(target_os = "macos")))]
mod unsupported_tests {
    use super::*;
    #[test]
    fn linux_refuses_before_any_filesystem_effect() {
        let recipe = crate::runtime::test_fixtures::recipe(
            &crate::runtime::test_fixtures::recipe_value("/missing/helper"),
        );
        let runtime = File::open("/dev/null").unwrap();
        use std::os::fd::AsFd;
        assert!(matches!(
            launch(
                recipe,
                runtime.as_fd(),
                Duration::ZERO,
                &AtomicBool::new(false)
            ),
            Err(LaunchFailure::Platform(PlatformRefusal::UnsupportedHost))
        ));
    }
}
