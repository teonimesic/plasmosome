use plasmosome_membrane::cell_args::CellArgs;
use plasmosome_membrane::daemon;
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

const USAGE: &str = "usage: membraned <config.json>
       membraned -h | --help | --version";

const HELP: &str = "membraned — membrane status daemon

usage: membraned <config.json>
       membraned -h | --help | --version

Runs in the foreground with required JSON configuration:
  {\"control_socket\":\"/private/path/membrane.uds\",\"status_deadline_ms\":500,\"brokers\":[]}

The configured Unix socket currently serves only membrane.status.
status_deadline_ms is a broker probe budget, not a strict elapsed-time bound.
SIGINT or SIGTERM requests a clean shutdown.
See the repository README for a bounded status-only demonstration.
";

extern "C" fn note_shutdown(_signal: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Relaxed);
}

#[allow(unsafe_code)]
fn stop_on(signal: libc::c_int) {
    // SAFETY: An all-zero libc::sigaction is a valid empty action with no flags. The handler only
    // stores to an atomic, which is async-signal-safe. Both pointers are to the local `action`,
    // which outlives the calls, and sigaction copies what it reads.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = note_shutdown as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

fn refuse(reason: String) -> ! {
    eprintln!("membraned: {reason}");
    std::process::exit(2)
}

fn main() {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let [path] = arguments.as_slice() else {
        if CellArgs::parse(&arguments).is_some() {
            refuse("cell mode is not built yet".to_string());
        }
        eprintln!("{USAGE}");
        std::process::exit(64);
    };
    if path == "-h" || path == "--help" {
        print!("{HELP}");
        return;
    }
    if path == "--version" {
        println!("membraned {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let path = Path::new(path);
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => refuse(format!("cannot read {}: {error}", path.display())),
    };
    let config = match daemon::parse_config(&text) {
        Ok(config) => config,
        Err(error) => refuse(format!("{}: {error}", path.display())),
    };

    stop_on(libc::SIGTERM);
    stop_on(libc::SIGINT);

    if let Err(error) = daemon::run(config, &SHUTDOWN) {
        eprintln!("membraned: {error}");
        std::process::exit(1);
    }
}
