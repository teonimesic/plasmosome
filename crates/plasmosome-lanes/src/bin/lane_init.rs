use std::process::ExitCode;

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("lane-init: refused: runs only as a Linux guest's process 1");
    ExitCode::from(2)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    if std::process::id() != 1 {
        eprintln!("lane-init: refused: runs only as process 1");
        return ExitCode::from(3);
    }
    linux::run();
    ExitCode::from(1)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use plasmosome_lanes::config::{GuestConfig, GuestMount, MountOption, Probe};
    use plasmosome_lanes::console::{BEGIN, END, ProbeResult, result_line};
    use plasmosome_lanes::guest_sys::{
        self, MS_NODEV, MS_NOEXEC, MS_NOSUID, MS_RDONLY, kill_all, power_off, reap_exited,
    };

    const CONFIG: &str = "/lane.json";
    const PROBE: &str = "/bin/lane-probe";
    const HOLD_DIR: &str = "/run/lane-hold";
    const MAX_PROBE_OUTPUT: u64 = 1024 * 1024;

    struct Console(Option<File>);

    impl Console {
        fn line(&mut self, text: &str) {
            if let Some(file) = self.0.as_mut() {
                let _ = file.write_all(format!("{text}\n").as_bytes());
            }
        }
    }

    fn open_console() -> Console {
        let open = |path: &str| OpenOptions::new().write(true).open(path).ok();
        Console(open("/dev/hvc0").or_else(|| open("/dev/console")))
    }

    fn start() -> Console {
        let _ = guest_sys::mount(
            "proc",
            Path::new("/proc"),
            "proc",
            MS_NOSUID | MS_NODEV | MS_NOEXEC,
        );
        let _ = guest_sys::mount("devtmpfs", Path::new("/dev"), "devtmpfs", MS_NOSUID);
        let _ = fs::write("/proc/sys/kernel/printk", "1 4 1 7");
        let mut console = open_console();
        let hold = fs::create_dir_all(HOLD_DIR)
            .and_then(|()| fs::set_permissions(HOLD_DIR, fs::Permissions::from_mode(0o1777)));
        if let Err(error) = hold {
            console.line(&format!("lane-init: {HOLD_DIR}: {error}"));
        }
        console
    }

    fn flags(options: &[MountOption]) -> libc::c_ulong {
        options.iter().fold(0, |flags, option| {
            flags
                | match option {
                    MountOption::Ro => MS_RDONLY,
                    MountOption::Nosuid => MS_NOSUID,
                    MountOption::Nodev => MS_NODEV,
                    MountOption::Noexec => MS_NOEXEC,
                }
        })
    }

    fn mount_share(mount: &GuestMount) -> Result<(), String> {
        fs::create_dir_all(&mount.path)
            .and_then(|()| {
                guest_sys::mount(&mount.tag, &mount.path, "virtiofs", flags(&mount.options))
            })
            .map_err(|error| {
                format!(
                    "lane-init: mount {} at {} failed: {error}",
                    mount.tag,
                    mount.path.display()
                )
            })
    }

    fn one_json_line(output: &[u8]) -> Option<serde_json::Value> {
        let text = std::str::from_utf8(output).ok()?;
        let line = text.strip_suffix('\n')?;
        if line.contains('\n') {
            return None;
        }
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .filter(serde_json::Value::is_object)
    }

    fn run_probe(probe: &Probe) -> ProbeResult {
        let mut result = ProbeResult {
            probe: probe.id.clone(),
            uid: probe.uid,
            exit: None,
            result: None,
        };
        let spawned = Command::new(PROBE)
            .args(&probe.args)
            .env_clear()
            .env("PATH", "/bin")
            .current_dir("/")
            .uid(probe.uid)
            .gid(probe.uid)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = spawned else {
            return result;
        };
        let mut output = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            let _ = stdout.take(MAX_PROBE_OUTPUT).read_to_end(&mut output);
        }
        result.exit = child.wait().ok().and_then(|status| status.code());
        result.result = one_json_line(&output);
        result
    }

    fn run_lane(console: &mut Console) {
        let config = match fs::read_to_string(CONFIG)
            .map_err(|error| error.to_string())
            .and_then(|text| GuestConfig::from_json(&text).map_err(|error| error.to_string()))
        {
            Ok(config) => config,
            Err(error) => {
                console.line(&format!("lane-init: refused: {CONFIG}: {error}"));
                return;
            }
        };
        for mount in &config.mounts {
            if let Err(error) = mount_share(mount) {
                console.line(&error);
                return;
            }
        }
        console.line(&format!("{BEGIN} {}", config.run_id));
        for probe in &config.probes {
            console.line(&result_line(&run_probe(probe)));
            reap_exited();
        }
        console.line(&format!("{END} {} {}", config.run_id, config.probes.len()));
    }

    pub fn run() {
        let mut console = start();
        run_lane(&mut console);
        kill_all();
        reap_exited();
        let error = power_off();
        console.line(&format!("lane-init: power off failed: {error}"));
    }
}
