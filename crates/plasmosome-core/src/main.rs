use plasmosome_core::daemon;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

const USAGE: &str = "usage: plasmosomed <config.json>
       plasmosomed -h | --help";

const HELP: &str = "plasmosomed — controller status daemon

usage: plasmosomed <config.json>
       plasmosomed -h | --help

Runs in the foreground with required JSON configuration:
  {\"control_socket\":\"/private/path/control.uds\",\"name\":\"quickstart\"}

The configured Unix socket currently serves only plasmosome.status.
SIGINT or SIGTERM requests a clean shutdown.
See the repository README for a bounded status-only demonstration.
";

extern "C" fn note_shutdown(_signal: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Relaxed);
}

fn stop_on(signal: libc::c_int) {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = note_shutdown as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

fn refuse(reason: String) -> ! {
    eprintln!("plasmosomed: {reason}");
    std::process::exit(2)
}

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(path), None) = (arguments.next(), arguments.next()) else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    if path == "-h" || path == "--help" {
        print!("{HELP}");
        return;
    }
    let path = Path::new(&path);
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
        eprintln!("plasmosomed: {error}");
        std::process::exit(1);
    }
}
