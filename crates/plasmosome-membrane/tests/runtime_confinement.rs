use plasmosome_membrane::runtime::confinement::{Confinement, Unverifiable, observe};
use plasmosome_membrane::runtime::digest::Digest;

#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "support/fixture.rs"]
mod fixture;

const HOST_POLICY: &str = "(version 1)
(deny default)
(allow file-read*)
(allow file-write* (subpath \"/usr/local/var/plasmosome/work\"))
(allow network-outbound (remote unix-socket))
(allow mach-lookup)
(allow mach-register)
(allow process-info*)
(allow sysctl-read)
(allow system-socket)
(deny file-write*)
(deny network*)
(deny process-exec)
";
const HOST_POLICY_SHA256: &str = "d4e3f9a296a4577f54bd51899d3bf6fac439e47e2b559c5df955825d60cd7c34";
const WORK_SUBPATH: &str = "(subpath \"/usr/local/var/plasmosome/work\")";

fn own_pid() -> libc::pid_t {
    libc::pid_t::try_from(std::process::id()).expect("this process's id fits a pid_t")
}

#[test]
fn the_policy_text_is_the_qualified_bundles_host_policy_without_its_final_nul() {
    let file = [HOST_POLICY.as_bytes(), b"\0"].concat();
    assert_eq!(Digest::of(&file).hex(), HOST_POLICY_SHA256);
    assert_eq!(HOST_POLICY.matches(WORK_SUBPATH).count(), 1);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn off_darwin_every_pid_is_unverifiable() {
    for pid in [own_pid(), 1, 0, -1] {
        assert_eq!(
            observe(pid),
            Confinement::Unverifiable {
                reason: Unverifiable::UnsupportedPlatform,
            },
            "pid {pid}"
        );
    }
}

#[cfg(target_os = "macos")]
mod darwin {
    use super::*;
    use plasmosome_membrane::runtime::confinement::PolicyBinding;
    use std::io::{BufRead, BufReader, Read};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::LazyLock;

    const FIVE: [&str; 5] = [
        "process-fork",
        "network-outbound",
        "network-inbound",
        "network-bind",
        "file-write-create",
    ];
    const SANDBOXED_OBSERVER: &str = "PLASMOSOME_TEST_SANDBOXED_OBSERVER";
    const OBSERVED_TWICE: &str = "observed twice from inside a sandbox";

    static SLEEPER: LazyLock<PathBuf> = LazyLock::new(|| {
        let executable = std::env::current_exe().expect("the running test executable has a path");
        fixture::compile_supervision_fixture(
            &fixture::fixture_cache_root(&executable)
                .with_file_name("plasmosome-sandboxed-sleeper"),
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/sandboxed_sleeper.c"),
        )
    });

    struct Sleeper {
        child: Child,
        _stdin: ChildStdin,
        unique_id: u64,
    }

    impl Sleeper {
        fn start(profile: &str) -> Sleeper {
            let mut child = Command::new(&*SLEEPER)
                .arg(profile)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("the sandboxed sleeper starts");
            let stdin = child.stdin.take().expect("the sleeper's stdin is piped");
            let stdout = child.stdout.take().expect("the sleeper's stdout is piped");
            let mut stderr = child.stderr.take().expect("the sleeper's stderr is piped");
            let mut sleeper = Sleeper {
                child,
                _stdin: stdin,
                unique_id: 0,
            };
            let mut line = String::new();
            BufReader::new(stdout)
                .read_line(&mut line)
                .expect("the sleeper's stdout is readable");
            match line
                .strip_prefix("ready ")
                .and_then(|rest| rest.strip_suffix('\n'))
                .and_then(|unique_id| unique_id.parse().ok())
            {
                Some(unique_id) => sleeper.unique_id = unique_id,
                None => {
                    let mut message = String::new();
                    stderr
                        .read_to_string(&mut message)
                        .expect("the sleeper's stderr is readable");
                    panic!(
                        "the sleeper applied {profile:?} and reported its unique id; it wrote {line:?} and {message:?}"
                    );
                }
            }
            sleeper
        }

        fn pid(&self) -> libc::pid_t {
            libc::pid_t::try_from(self.child.id()).expect("a child's id fits a pid_t")
        }
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn observed_under(profile: &str) -> Confinement {
        let sleeper = Sleeper::start(profile);
        observe(sleeper.pid())
    }

    fn allowing_every_required_operation(sandboxed: bool) -> Confinement {
        Confinement::Unconfined {
            sandboxed,
            allowed: FIVE.to_vec(),
        }
    }

    fn allowing_only(operation: &str) -> Confinement {
        observed_under(&format!(
            "(version 1)(deny default)(allow file-read*)(allow {operation})"
        ))
    }

    fn sandboxed_allowing(allowed: &[&'static str]) -> Confinement {
        Confinement::Unconfined {
            sandboxed: true,
            allowed: allowed.to_vec(),
        }
    }

    fn gone() -> Confinement {
        Confinement::Unverifiable {
            reason: Unverifiable::ProcessUnreadable { errno: libc::ESRCH },
        }
    }

    #[allow(unsafe_code)]
    fn wait_until_exited_unreaped(pid: libc::pid_t) {
        let id = libc::id_t::try_from(pid).expect("a child's pid is positive");
        loop {
            // SAFETY: an all-zero siginfo_t is a valid value of this plain C struct.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: `info` is a local siginfo_t the call may write. WNOWAIT leaves the child
            // waitable, so its std Child still owns the reap.
            let waited =
                unsafe { libc::waitid(libc::P_PID, id, &mut info, libc::WEXITED | libc::WNOWAIT) };
            if waited == 0 {
                return;
            }
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::Interrupted,
                "waitid sees the sleeper exit: {error}"
            );
        }
    }

    #[test]
    fn the_qualified_host_policy_is_confined() {
        let work = tempfile::tempdir().expect("a temporary work directory is created");
        let work = work
            .path()
            .canonicalize()
            .expect("the work directory has a canonical path");
        let profile =
            HOST_POLICY.replace(WORK_SUBPATH, &format!("(subpath \"{}\")", work.display()));
        assert!(profile.contains(&work.display().to_string()));
        let sleeper = Sleeper::start(&profile);
        match observe(sleeper.pid()) {
            Confinement::Confined(evidence) => {
                assert_eq!(evidence.pid(), sleeper.pid());
                assert_eq!(
                    evidence.unique_id(),
                    sleeper.unique_id,
                    "the evidence names the process by the unique id it reads for itself"
                );
                assert_eq!(evidence.denied(), FIVE);
                assert_eq!(evidence.binding(), PolicyBinding::PropertiesObserved);
            }
            other => panic!("the qualified host policy is confined, not {other:?}"),
        }
    }

    #[test]
    fn an_allow_default_sandbox_is_unconfined() {
        assert_eq!(
            observed_under("(version 1)(allow default)"),
            allowing_every_required_operation(true)
        );
    }

    #[test]
    fn a_sandbox_allowing_process_fork_is_unconfined() {
        assert_eq!(
            allowing_only("process-fork"),
            sandboxed_allowing(&["process-fork"])
        );
    }

    #[test]
    fn a_sandbox_allowing_network_outbound_is_unconfined() {
        assert_eq!(
            allowing_only("network-outbound"),
            sandboxed_allowing(&["network-outbound"])
        );
    }

    #[test]
    fn a_sandbox_allowing_network_inbound_is_unconfined_and_allows_network_bind_too() {
        assert_eq!(
            allowing_only("network-inbound"),
            sandboxed_allowing(&["network-inbound", "network-bind"]),
            "the sandbox answers network-bind allowed whenever network-inbound is allowed"
        );
    }

    #[test]
    fn a_sandbox_allowing_network_bind_is_unconfined() {
        assert_eq!(
            allowing_only("network-bind"),
            sandboxed_allowing(&["network-bind"])
        );
    }

    #[test]
    fn a_sandbox_allowing_file_write_create_is_unconfined() {
        assert_eq!(
            allowing_only("file-write-create"),
            sandboxed_allowing(&["file-write-create"])
        );
    }

    #[test]
    fn a_process_with_no_sandbox_is_unconfined() {
        assert_eq!(
            observed_under("none"),
            allowing_every_required_operation(false)
        );
    }

    #[test]
    fn a_bare_deny_default_sandbox_is_confined_too_so_the_policy_is_not_identified() {
        assert!(
            matches!(
                observed_under("(version 1)(deny default)"),
                Confinement::Confined(evidence)
                    if evidence.binding() == PolicyBinding::PropertiesObserved
            ),
            "observe checks properties, so a policy other than the verified one can pass"
        );
    }

    #[test]
    fn an_exited_process_not_yet_reaped_is_unverifiable() {
        let mut sleeper = Sleeper::start("none");
        let pid = sleeper.pid();
        sleeper.child.kill().expect("the sleeper is signalled");
        wait_until_exited_unreaped(pid);
        assert_eq!(
            observe(pid),
            gone(),
            "a zombie answers 1 to every sandbox query, so those answers must not count"
        );
    }

    #[test]
    fn a_reaped_pid_is_unverifiable() {
        let mut sleeper = Sleeper::start("none");
        let pid = sleeper.pid();
        sleeper.child.kill().expect("the sleeper is signalled");
        sleeper.child.wait().expect("the sleeper is reaped");
        assert_eq!(observe(pid), gone());
    }

    #[test]
    fn this_test_process_runs_with_no_sandbox_and_is_unconfined() {
        assert_eq!(
            observe(own_pid()),
            allowing_every_required_operation(false),
            "these tests must run outside any sandbox"
        );
    }

    #[test]
    #[ignore = "run inside sandbox-exec by a_sandboxed_observer_trusts_no_answer_on_any_call"]
    fn observe_twice_from_inside_a_sandbox() {
        let Some(helper) = std::env::var_os(SANDBOXED_OBSERVER) else {
            return;
        };
        let helper: libc::pid_t = helper
            .to_str()
            .and_then(|helper| helper.parse().ok())
            .expect("the outer test passes the helper's pid");
        for call in ["first", "second"] {
            for pid in [own_pid(), helper] {
                assert_eq!(
                    observe(pid),
                    Confinement::Unverifiable {
                        reason: Unverifiable::SupervisorSandboxed,
                    },
                    "the {call} observation of pid {pid}"
                );
            }
        }
        println!("{OBSERVED_TWICE}");
    }

    #[test]
    fn a_sandboxed_observer_trusts_no_answer_on_any_call() {
        let helper = Sleeper::start("(version 1)(deny default)");
        let output = Command::new("/usr/bin/sandbox-exec")
            .args(["-p", "(version 1)(allow default)"])
            .arg(std::env::current_exe().expect("the running test executable has a path"))
            .args([
                "--exact",
                "darwin::observe_twice_from_inside_a_sandbox",
                "--include-ignored",
                "--nocapture",
            ])
            .env(SANDBOXED_OBSERVER, helper.pid().to_string())
            .stdin(Stdio::null())
            .output()
            .expect("sandbox-exec runs this test executable");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains(OBSERVED_TWICE),
            "the observation inside sandbox-exec ended {}; stdout {stdout:?}; stderr {:?}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
