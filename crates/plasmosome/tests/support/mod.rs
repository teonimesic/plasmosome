use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use plasmosome_core::daemon;
use serde_json::json;
use tempfile::TempDir;

const PATIENCE: Duration = Duration::from_secs(10);

/// The directory private roots are made in: the canonical per-user temporary directory on macOS,
/// and `XDG_RUNTIME_DIR`, else the target's tmp, on Linux. Every ancestor passes the path checks.
#[cfg(target_os = "macos")]
pub fn base() -> PathBuf {
    let base = std::env::temp_dir();
    base.canonicalize()
        .unwrap_or_else(|error| panic!("{} does not resolve: {error}", base.display()))
}

#[cfg(not(target_os = "macos"))]
pub fn base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|base| !base.is_empty())
        .map_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")), PathBuf::from)
}

/// A `0700` directory under `base`, removed when it drops.
pub fn private_dir(base: &Path) -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("pb")
        .tempdir_in(base)
        .unwrap_or_else(|error| panic!("no directory under {}: {error}", base.display()));
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).expect("0700");
    dir
}

/// `plasmosome-core`'s daemon loop on a thread, configured with `name`, serving `socket`. It
/// stops when it drops.
pub struct Daemon {
    stop: &'static AtomicBool,
    thread: Option<JoinHandle<()>>,
}

impl Daemon {
    pub fn start(name: &str, root: &Path, socket: &Path) -> Daemon {
        let config = json!({
            "control_socket": socket,
            "name": name,
            "instance_root": root.join("instance"),
            "recovery_deadline_ms": 5000,
        });
        let config = daemon::parse_config(&config.to_string()).expect("a valid configuration");
        let stop: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        let thread = thread::spawn(move || {
            daemon::run(config, stop).expect("plasmosomed serves until it is stopped");
        });
        let until = Instant::now() + PATIENCE;
        while !socket.exists() {
            assert!(
                Instant::now() < until,
                "plasmosomed did not bind {socket:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
        Daemon {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let until = Instant::now() + Duration::from_secs(2);
            while !thread.is_finished() && Instant::now() < until {
                thread::sleep(Duration::from_millis(5));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}
