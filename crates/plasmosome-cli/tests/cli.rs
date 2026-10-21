use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::OsStringExt;

use plasmosome_cli::json::Json;
use plasmosome_cli::{
    COMMANDS, ClientCode, Command, Exit, Form, Invocation, Reading, handle, read,
};

const SPEC_025: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/025-plasmosome-command-line.md"
);

const COMMAND_TABLE_HEADER: &str = "| Command | Wire method | Changes state | Served |";

const EPIPE: i32 = 32;

fn args(line: &[&str]) -> Vec<OsString> {
    line.iter().map(OsString::from).collect()
}

fn not_utf8() -> OsString {
    OsString::from_vec(vec![b'a', 0xff])
}

fn escape_character() -> char {
    char::from_u32(0x1b).expect("ESC is a code point")
}

fn status_served() -> Vec<Command> {
    COMMANDS
        .iter()
        .map(|command| Command {
            served: command.command == "plasmosome.status",
            ..*command
        })
        .collect()
}

fn status() -> Command {
    status_served()[0]
}

fn spec_025_commands() -> Vec<(Vec<String>, String, bool, bool)> {
    let spec = fs::read_to_string(SPEC_025).expect("spec 025 is readable beside the crate");
    let mut commands = Vec::new();
    for row in spec
        .lines()
        .skip_while(|line| *line != COMMAND_TABLE_HEADER)
        .skip(2)
        .take_while(|line| line.starts_with('|'))
    {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let names: Vec<&str> = cells[1]
            .split(", ")
            .map(|name| name.trim_matches('`'))
            .collect();
        if names[0].starts_with("plasmid ") {
            continue;
        }
        let registry = names[0].starts_with("plasmosome registry ");
        for name in names {
            let name = name.strip_prefix("plasmosome ").unwrap_or(name);
            let name = if registry {
                name.strip_prefix("registry ").unwrap_or(name)
            } else {
                name
            };
            let mut words: Vec<String> = name.split(' ').map(str::to_string).collect();
            let (command, is_method, changes_state) = if registry {
                let changes = cells[3].contains(&format!("`{name}`"));
                (words.join("."), false, changes)
            } else {
                (
                    cells[2].trim_matches('`').to_string(),
                    true,
                    cells[3] == "yes",
                )
            };
            if registry {
                words.insert(0, "registry".to_string());
            }
            commands.push((words, command, is_method, changes_state));
        }
    }
    commands
}

fn refused(reading: &Reading) -> (&str, ClientCode, Option<String>) {
    let Reading::Refuse { command, error, .. } = reading else {
        panic!("expected a refusal, read {reading:?}");
    };
    let argument = error
        .fields()
        .iter()
        .find(|(key, _)| key == "argument")
        .map(|(_, value)| match value {
            Json::String(text) => text.clone(),
            other => panic!("`argument` is not a string: {other:?}"),
        });
    (command, error.code(), argument)
}

fn assert_usage(line: Vec<OsString>, commands: &[Command], argument: Option<&str>) {
    let reading = read(&line, commands);
    let (command, code, given) = refused(&reading);
    assert_eq!(
        (command, code, given.as_deref()),
        ("invalid", ClientCode::InvalidArgument, argument),
        "{line:?} read as {reading:?}"
    );
}

fn envelope(stderr: &[u8]) -> serde_json::Value {
    let text = std::str::from_utf8(stderr).expect("stderr is UTF-8");
    assert!(
        text.ends_with('\n') && text.matches('\n').count() == 1,
        "stderr is not one line: {text:?}"
    );
    serde_json::from_str(text).expect("stderr is one JSON envelope")
}

struct Closed;

impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from_raw_os_error(EPIPE))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_command_table_is_spec_025s() {
    let spec = spec_025_commands();
    assert_eq!(
        spec.len(),
        23,
        "spec 025's command table was not read whole: {spec:?}"
    );
    let built: Vec<(Vec<String>, String, bool, bool)> = COMMANDS
        .iter()
        .map(|command| {
            (
                command.words.iter().map(|word| word.to_string()).collect(),
                command.command.to_string(),
                command.is_method,
                command.changes_state,
            )
        })
        .collect();
    assert_eq!(built, spec);
    assert!(
        COMMANDS.iter().all(|command| !command.served),
        "this build serves no command yet; plasmosome-l1k6 marks status served when it works"
    );
}

#[test]
fn help_comes_before_every_other_rule() {
    for line in [
        &["--help"][..],
        &["-h"],
        &["cell", "list", "--help"],
        &["--help", "--version"],
        &["status", "--version", "-h"],
        &["--kernel", "--help"],
        &["cell", "list", "--json", "--human", "--frob", "--help"],
        &["germinate", "--help"],
        &["germinate", "g", "--help"],
        &["registry", "source", "--help"],
        &["cell", "-h"],
    ] {
        let reading = read(&args(line), COMMANDS);
        assert!(
            matches!(reading, Reading::Help { .. }),
            "{line:?} read as {reading:?}"
        );
    }
    let reading = read(&args(&["cell", "exec", "--", "--help"]), COMMANDS);
    let (command, code, _) = refused(&reading);
    assert_eq!((command, code), ("cell.exec", ClientCode::NotServed));
}

#[test]
fn help_after_words_that_name_no_command_is_a_usage_error() {
    assert_usage(
        args(&["frobnicate", "--help"]),
        COMMANDS,
        Some("frobnicate"),
    );
    assert_usage(args(&["cell", "frob", "--help"]), COMMANDS, Some("frob"));
    assert_usage(
        args(&["status", "extra", "--help"]),
        COMMANDS,
        Some("extra"),
    );
    assert_usage(
        args(&["germinate", "g", "h", "--help"]),
        COMMANDS,
        Some("h"),
    );
}

#[test]
fn the_version_is_read_only_alone() {
    assert_eq!(read(&args(&["--version"]), COMMANDS), Reading::Version);
    for line in [
        &["status", "--version"][..],
        &["--version", "--json"],
        &["--version", "--version"],
        &["cell", "list", "--version"],
    ] {
        assert_usage(args(line), COMMANDS, Some("--version"));
    }
}

#[test]
fn an_unserved_command_reads_only_the_form_flags() {
    let reading = read(
        &args(&[
            "cell",
            "list",
            "--kernel",
            "../x",
            "--frob",
            "--deadline-ms",
            "0",
            "x",
        ]),
        COMMANDS,
    );
    let (command, code, _) = refused(&reading);
    assert_eq!((command, code), ("cell.list", ClientCode::NotServed));
    let Reading::Refuse { form, .. } = read(&args(&["cell", "list", "--human"]), COMMANDS) else {
        panic!("cell list is refused");
    };
    assert_eq!(form, Form::Human);
    assert_usage(
        args(&["cell", "list", "--json", "--human"]),
        COMMANDS,
        Some("--human"),
    );
    assert_usage(
        args(&["cell", "list", "--human", "--json"]),
        COMMANDS,
        Some("--json"),
    );
    assert_usage(
        args(&["cell", "list", "--json", "--json"]),
        COMMANDS,
        Some("--json"),
    );
    assert_usage(
        args(&["cell", "list", "--human", "--human"]),
        COMMANDS,
        Some("--human"),
    );
}

#[test]
fn the_full_parse_refuses_each_fault() {
    let commands = status_served();
    let escaped_name = format!("a{}[31m", escape_character());
    let forty_digits = "1".repeat(40);
    let lossy = format!("a{}", char::REPLACEMENT_CHARACTER);
    let cases: Vec<(Vec<OsString>, Option<&str>)> = vec![
        (
            args(&["status", "--kernel", "a", "--kernel", "b"]),
            Some("--kernel"),
        ),
        (
            args(&[
                "--socket",
                "/a/control.uds",
                "status",
                "--socket",
                "/b/control.uds",
            ]),
            Some("--socket"),
        ),
        (
            args(&["status", "--deadline-ms", "5", "--deadline-ms", "6"]),
            Some("--deadline-ms"),
        ),
        (args(&["status", "--kernel", "../x"]), Some("../x")),
        (args(&["status", "--kernel", "a/b"]), Some("a/b")),
        (args(&["status", "--kernel", ".."]), Some("..")),
        (args(&["status", "--kernel", ""]), Some("")),
        (
            args(&["status", "--kernel", &escaped_name]),
            Some(escaped_name.as_str()),
        ),
        (
            args(&["status", "--socket", "relative/control.uds"]),
            Some("relative/control.uds"),
        ),
        (
            args(&["status", "--socket", "/a//b/control.uds"]),
            Some("/a//b/control.uds"),
        ),
        (
            args(&["status", "--socket", "/a/./control.uds"]),
            Some("/a/./control.uds"),
        ),
        (
            args(&["status", "--socket", "/a/../control.uds"]),
            Some("/a/../control.uds"),
        ),
        (args(&["status", "--socket", "/a/b/"]), Some("/a/b/")),
        (args(&["status", "--socket", "/"]), Some("/")),
        (args(&["status", "--deadline-ms", "0"]), Some("0")),
        (
            args(&["status", "--deadline-ms", "3600001"]),
            Some("3600001"),
        ),
        (
            args(&["status", "--deadline-ms", &forty_digits]),
            Some(forty_digits.as_str()),
        ),
        (args(&["status", "--deadline-ms", "+5"]), Some("+5")),
        (args(&["status", "--deadline-ms", "-5"]), Some("-5")),
        (args(&["status", "--deadline-ms", ""]), Some("")),
        (args(&["status", "--frob"]), Some("--frob")),
        (
            args(&["status", "--trusted-uid", "0"]),
            Some("--trusted-uid"),
        ),
        (args(&["status", "--json", "--human"]), Some("--human")),
        (args(&["status", "--human", "--json"]), Some("--json")),
        (args(&["status", "--json", "--json"]), Some("--json")),
        (args(&["status", "extra"]), Some("extra")),
        (args(&["status", "--kernel"]), Some("--kernel")),
        (args(&["status", "--", "x"]), Some("--")),
        (vec!["status".into(), not_utf8()], Some(lossy.as_str())),
        (
            vec!["status".into(), "--".into(), not_utf8()],
            Some(lossy.as_str()),
        ),
        (vec![not_utf8()], Some(lossy.as_str())),
        (args(&[]), None),
        (args(&["--json"]), None),
        (args(&["frobnicate"]), Some("frobnicate")),
        (args(&["cell"]), Some("cell")),
        (args(&["cell", "frob"]), Some("frob")),
        (args(&["registry", "source"]), Some("source")),
    ];
    for (line, argument) in cases {
        assert_usage(line, &commands, argument);
    }
}

#[test]
fn the_full_parse_reads_each_value() {
    let commands = status_served();
    let line = args(&[
        "--kernel",
        "a",
        "status",
        "--socket",
        "/x/control.uds",
        "--deadline-ms",
        "3600000",
        "--human",
    ]);
    assert_eq!(
        read(&line, &commands),
        Reading::Run(Invocation {
            command: status(),
            kernel: Some("a".to_string()),
            socket: Some("/x/control.uds".to_string()),
            deadline_ms: Some(3_600_000),
            form: Form::Human,
        })
    );
    assert_eq!(
        read(
            &args(&["status", "--deadline-ms", "1", "--kernel", "-a"]),
            &commands
        ),
        Reading::Run(Invocation {
            command: status(),
            kernel: Some("-a".to_string()),
            socket: None,
            deadline_ms: Some(1),
            form: Form::Json,
        })
    );
    assert_eq!(
        read(&args(&["status", "--json"]), &commands),
        Reading::Run(Invocation {
            command: status(),
            kernel: None,
            socket: None,
            deadline_ms: None,
            form: Form::Json,
        })
    );
}

#[test]
fn a_usage_error_uses_the_form_asked_for() {
    for (line, expected) in [
        (&["frobnicate"][..], Form::Json),
        (&["frobnicate", "--human"], Form::Human),
        (&["frobnicate", "--human", "--human"], Form::Json),
        (&["frobnicate", "--json", "--human"], Form::Json),
    ] {
        let Reading::Refuse { form, .. } = read(&args(line), COMMANDS) else {
            panic!("{line:?} is refused");
        };
        assert_eq!(form, expected, "{line:?}");
    }
}

#[test]
fn an_escaped_echo_in_both_forms() {
    let commands = status_served();
    let escape = escape_character();
    let name = format!("a{escape}[31m");
    for (form, shown) in [
        (None, format!("{}u001b", '\\')),
        (Some("--human"), "<U+001B>".to_string()),
    ] {
        let mut line = args(&["status", "--kernel", &name]);
        line.extend(form.map(OsString::from));
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let exit = handle(&line, "0.0.0", &commands, &mut stdout, &mut stderr);
        assert_eq!(exit, Exit::Usage);
        assert!(stdout.is_empty());
        let text = String::from_utf8(stderr).expect("stderr is UTF-8");
        assert!(!text.contains(escape), "a raw ESC reached stderr: {text:?}");
        assert!(
            text.contains(&shown),
            "{text:?} does not show ESC as {shown:?}"
        );
    }
}

#[test]
fn a_failed_stdout_is_local_io() {
    for (line, command) in [(&["--help"][..], "--help"), (&["--version"], "--version")] {
        let mut stderr = Vec::new();
        let exit = handle(&args(line), "0.0.0", COMMANDS, &mut Closed, &mut stderr);
        assert_eq!(exit, Exit::Unavailable, "{line:?}");
        let envelope = envelope(&stderr);
        assert_eq!(envelope["command"], command);
        assert_eq!(envelope["error"]["code"], "local_io");
        assert_eq!(envelope["error"]["errno"], EPIPE);
    }
    let mut stderr = Vec::new();
    let exit = handle(
        &args(&["--help", "--human"]),
        "0.0.0",
        COMMANDS,
        &mut Closed,
        &mut stderr,
    );
    assert_eq!(exit, Exit::Unavailable);
    let text = String::from_utf8(stderr).expect("stderr is UTF-8");
    assert!(text.ends_with("(client local_io)\n"), "{text:?}");
}

#[test]
fn a_served_command_with_no_code_is_internal() {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit = handle(
        &args(&["status"]),
        "0.0.0",
        &status_served(),
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(exit, Exit::Internal);
    assert!(stdout.is_empty());
    let envelope = envelope(&stderr);
    assert_eq!(envelope["command"], "plasmosome.status");
    assert_eq!(envelope["error"]["code"], "internal");
}

fn help_text(line: &[&str], commands: &[Command]) -> String {
    match read(&args(line), commands) {
        Reading::Help { text, .. } => text,
        other => panic!("{line:?} read as {other:?}"),
    }
}

#[test]
fn help_says_what_each_command_is() {
    let commands = status_served();
    let help = help_text(&["--help"], &commands);
    let row = help
        .lines()
        .find(|line| line.starts_with("  plasmosome status "))
        .expect("status is listed");
    assert!(
        row.ends_with("  served") && !row.ends_with("not served"),
        "{row:?}"
    );
    assert!(help_text(&["status", "--help"], &commands).contains("\nServed in this build.\n"));
    let search = help_text(&["registry", "search", "--help"], COMMANDS);
    assert!(
        search.contains("Spec 020's registry command search, which changes no state."),
        "{search}"
    );
    assert!(
        help_text(&["registry", "yank", "--help"], COMMANDS).contains("which changes state."),
        "yank changes state"
    );
}
