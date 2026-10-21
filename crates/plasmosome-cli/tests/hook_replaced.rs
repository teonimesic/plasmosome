use std::ffi::OsString;
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::sync::{Mutex, PoisonError};

use plasmosome_cli::{COMMANDS, Context, Env, Exit, SystemClock, handle};

struct NoVariables;

impl Env for NoVariables {
    fn home(&self) -> Option<OsString> {
        None
    }

    fn kernel(&self) -> Option<OsString> {
        None
    }
}

const CONTEXT: Context<'static> = Context {
    version: "0.0.0",
    commands: COMMANDS,
    trusted_uid: 0,
    clock: &SystemClock,
    env: &NoVariables,
};

const IN_A_COMMAND: &str = "a panic inside a command, after a newer hook was set";

static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn message(info: &PanicHookInfo<'_>) -> String {
    info.payload_as_str().unwrap_or_default().to_string()
}

fn args(line: &[&str]) -> Vec<OsString> {
    line.iter().map(OsString::from).collect()
}

struct PanicsOnWrite;

impl Write for PanicsOnWrite {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("{IN_A_COMMAND}")
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_hook_set_after_the_first_call_replaces_the_quiet_one() {
    let first = handle(
        &args(&["--version"]),
        &CONTEXT,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(first, Exit::Done);
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        SEEN.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message(info));
        previous(info);
    }));
    let exit = handle(
        &args(&["--help"]),
        &CONTEXT,
        &mut PanicsOnWrite,
        &mut Vec::new(),
    );
    assert_eq!(exit, Exit::Internal);
    let seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        seen,
        [IN_A_COMMAND],
        "handle wrapped the hook set after its first call"
    );
}
