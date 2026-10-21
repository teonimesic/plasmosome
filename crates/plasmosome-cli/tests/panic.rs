use std::ffi::OsString;
use std::hint::black_box;
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::sync::{Mutex, Once, PoisonError};
use std::thread;

use plasmosome_cli::{COMMANDS, Exit, handle};

const IN_A_COMMAND: &str = "a panic inside a command";

const IN_A_FORM: &str = "a panic inside a command with a form";

const OUTSIDE: &str = "a panic outside a command";

const ON_A_HELPER: &str = "a panic on a helper thread";

const NESTING_BEFORE: &str = "a panic before many calls";

const NESTING_AFTER: &str = "a panic after many calls";

const CALLS: usize = 5_000;

const SAME_DEPTH: usize = 4_096;

static SEEN: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());

fn record_panics() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let marker = 0_u8;
            let depth = std::ptr::from_ref(black_box(&marker)).addr();
            SEEN.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((message(info), depth));
            previous(info);
        }));
    });
}

fn message(info: &PanicHookInfo<'_>) -> String {
    info.payload_as_str().unwrap_or_default().to_string()
}

fn seen(text: &str) -> Vec<usize> {
    SEEN.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(message, _)| message == text)
        .map(|(_, depth)| *depth)
        .collect()
}

fn args(line: &[&str]) -> Vec<OsString> {
    line.iter().map(OsString::from).collect()
}

fn version() -> Exit {
    handle(
        &args(&["--version"]),
        "0.0.0",
        COMMANDS,
        &mut Vec::new(),
        &mut Vec::new(),
    )
}

fn panic_outside(text: &'static str) -> usize {
    assert!(panic::catch_unwind(|| panic!("{text}")).is_err());
    let depths = seen(text);
    assert_eq!(
        depths.len(),
        1,
        "the hook set before handle saw {text:?} {} times",
        depths.len()
    );
    depths[0]
}

struct PanicsOnWrite(&'static str);

impl Write for PanicsOnWrite {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("{}", self.0)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct PanicsOnAHelperThread(Vec<u8>);

impl Write for PanicsOnAHelperThread {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let helper = thread::spawn(|| panic!("{ON_A_HELPER}"));
        assert!(helper.join().is_err());
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_panic_in_a_command_is_internal_and_no_other_panic_is_hidden() {
    record_panics();
    let mut stderr = Vec::new();
    let exit = handle(
        &args(&["cell", "list", "--help"]),
        "0.0.0",
        COMMANDS,
        &mut PanicsOnWrite(IN_A_COMMAND),
        &mut stderr,
    );
    assert_eq!(exit, Exit::Internal);
    assert!(
        seen(IN_A_COMMAND).is_empty(),
        "the panic reached the hook that prints its message"
    );
    let text = String::from_utf8(stderr).expect("stderr is UTF-8");
    assert!(
        text.ends_with('\n') && text.matches('\n').count() == 1,
        "{text:?}"
    );
    assert!(
        !text.contains(IN_A_COMMAND),
        "the panic's text reached stderr: {text:?}"
    );
    let envelope: serde_json::Value =
        serde_json::from_str(&text).expect("stderr is one JSON envelope");
    assert_eq!(envelope["command"], "--help");
    assert_eq!(envelope["error"]["domain"], "client");
    assert_eq!(envelope["error"]["code"], "internal");
    assert!(envelope.get("instance").is_none());

    panic_outside(OUTSIDE);
}

#[test]
fn a_panic_writes_internal_in_the_form_the_line_chose() {
    record_panics();
    for (line, human) in [
        (&["--help"][..], false),
        (&["--help", "--human"], true),
        (&["--help", "--human", "--json"], false),
    ] {
        let mut stderr = Vec::new();
        let exit = handle(
            &args(line),
            "0.0.0",
            COMMANDS,
            &mut PanicsOnWrite(IN_A_FORM),
            &mut stderr,
        );
        assert_eq!(exit, Exit::Internal, "{line:?}");
        let text = String::from_utf8(stderr).expect("stderr is UTF-8");
        let parsed = serde_json::from_str::<serde_json::Value>(&text);
        assert!(
            seen(IN_A_FORM).is_empty(),
            "{line:?}: the panic reached the hook"
        );
        if human {
            assert!(
                text.ends_with("\n(client internal)\n"),
                "{line:?}: {text:?}"
            );
            assert!(parsed.is_err(), "{line:?}: {text:?}");
        } else {
            let envelope = parsed.expect("stderr is one JSON envelope");
            assert_eq!(envelope["error"]["code"], "internal", "{line:?}");
        }
    }
}

#[test]
fn a_panic_on_another_thread_while_a_command_runs_reaches_the_earlier_hook() {
    record_panics();
    let mut stdout = PanicsOnAHelperThread(Vec::new());
    let exit = handle(
        &args(&["--version"]),
        "0.0.0",
        COMMANDS,
        &mut stdout,
        &mut Vec::new(),
    );
    assert_eq!(exit, Exit::Done);
    assert_eq!(stdout.0, b"plasmosome 0.0.0\n");
    assert!(
        !seen(ON_A_HELPER).is_empty(),
        "a helper thread's panic during handle never reached the hook set before it"
    );
}

#[test]
fn many_calls_install_one_hook() {
    record_panics();
    assert_eq!(version(), Exit::Done);
    let before = panic_outside(NESTING_BEFORE);
    for _ in 0..CALLS {
        assert_eq!(version(), Exit::Done);
    }
    let after = panic_outside(NESTING_AFTER);
    assert!(
        before.abs_diff(after) < SAME_DEPTH,
        "after {CALLS} calls the hook ran {} bytes deeper in the stack",
        before.abs_diff(after)
    );
}
