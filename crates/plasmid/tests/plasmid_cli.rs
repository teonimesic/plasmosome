use std::ffi::OsStr;
use std::process::{Command, Output};

mod support;

use support::{entries, envelope, text, usage_error};

const UNSERVED: [(&str, &[&str], &str); 5] = [
    ("list", &[], "plasmid.list"),
    ("add", &["x"], "plasmid.add"),
    ("remove", &["x"], "plasmid.remove"),
    ("reload", &["x"], "plasmid.reload"),
    ("renew", &["x"], "plasmid.renew"),
];

const COMMANDS: [(&str, bool); 6] = [
    ("new", true),
    ("list", false),
    ("add", false),
    ("remove", false),
    ("reload", false),
    ("renew", false),
];

const GLOBAL_FLAGS: [&str; 8] = [
    "--kernel",
    "--socket",
    "--deadline-ms",
    "--json",
    "--human",
    "-h",
    "--help",
    "--version",
];

fn plasmid() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plasmid"));
    command.env_remove("PLASMOSOME_KERNEL");
    command
}

fn run_with<S: AsRef<OsStr> + std::fmt::Debug>(mut command: Command, arguments: &[S]) -> Output {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let output = command
        .current_dir(scratch.path())
        .args(arguments)
        .output()
        .expect("the plasmid binary runs");
    assert_eq!(
        entries(scratch.path()),
        Vec::<String>::new(),
        "plasmid wrote into its working directory for {arguments:?}"
    );
    output
}

fn run<S: AsRef<OsStr> + std::fmt::Debug>(arguments: &[S]) -> Output {
    run_with(plasmid(), arguments)
}

fn closed_pipe() -> std::io::PipeWriter {
    let (reader, writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    writer
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| character.is_whitespace() || character == ',')
}

fn binary_help() -> String {
    let output = run(&["--help"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    text(&output.stdout)
}

fn assert_not_served(output: &Output, method: &str) {
    assert_eq!(output.status.code(), Some(2), "{method}: {output:?}");
    let (command, error) = envelope(output);
    assert_eq!(command, method, "{output:?}");
    assert_eq!(error["code"], "not_served", "{output:?}");
    assert_eq!(error["method"], method, "{output:?}");
    let mut keys: Vec<&str> = error.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["code", "domain", "message", "method"], "{output:?}");
}

#[test]
fn version_is_the_sole_argument() {
    let expected = format!("plasmid {}\n", env!("CARGO_PKG_VERSION"));
    let mut hostile = plasmid();
    hostile.env("PLASMOSOME_KERNEL", "../x").env_remove("HOME");
    for command in [plasmid(), hostile] {
        let output = run_with(command, &["--version"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(text(&output.stdout), expected);
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    for arguments in [
        &["--version", "extra"][..],
        &["new", "demo", "--version"],
        &["add", "x", "--version"],
        &["--version", "--json"],
    ] {
        let error = usage_error(&run(arguments));
        assert_eq!(error["argument"], "--version", "{arguments:?}");
    }
    let error = usage_error(&run(&["--", "--version"]));
    assert_eq!(error["argument"], "--");
    let output = run(&["--version", "--help"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(text(&output.stdout), binary_help());
}

#[test]
fn help_lists_every_command_with_its_mark() {
    for flag in ["--help", "-h"] {
        let output = run(&[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag}: {output:?}");
        assert!(output.stderr.is_empty(), "{flag}: {output:?}");
        let stdout = text(&output.stdout);
        for (command, served) in COMMANDS {
            let lines: Vec<&str> = stdout
                .lines()
                .filter(|line| {
                    line.strip_prefix("  ")
                        .and_then(|rest| rest.split_whitespace().next())
                        == Some(command)
                })
                .collect();
            assert_eq!(lines.len(), 1, "one line for {command}: {stdout}");
            if served {
                assert!(
                    lines[0].contains("served") && !lines[0].contains("not served"),
                    "{command} is marked served: {stdout}"
                );
            } else {
                assert!(
                    lines[0].contains("not served"),
                    "{command} is marked not served: {stdout}"
                );
            }
        }
        for named in GLOBAL_FLAGS.into_iter().chain(["PLASMOSOME_KERNEL"]) {
            assert!(
                tokens(&stdout).any(|token| token == named),
                "{named}: {stdout}"
            );
        }
        let (_, table) = stdout
            .split_once("\nExit codes:\n")
            .unwrap_or_else(|| panic!("a line `Exit codes:`: {stdout}"));
        let firsts: Vec<&str> = table
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        for code in ["0", "1", "2", "3", "64", "70"] {
            assert_eq!(
                firsts.iter().filter(|first| **first == code).count(),
                1,
                "exit {code} has one line: {stdout}"
            );
        }
    }
}

#[test]
fn a_command_help_says_whether_it_is_served() {
    let binary = binary_help();
    for (arguments, method) in [
        (&["add", "x", "--help"][..], "plasmid.add"),
        (&["add", "--help"], "plasmid.add"),
        (&["list", "--help"], "plasmid.list"),
        (&["remove", "x", "--help"], "plasmid.remove"),
        (&["reload", "x", "-h"], "plasmid.reload"),
        (&["renew", "x", "--help"], "plasmid.renew"),
        (&["--help", "add", "x"], "plasmid.add"),
    ] {
        let output = run(arguments);
        assert_eq!(output.status.code(), Some(0), "{arguments:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{arguments:?}: {output:?}");
        let stdout = text(&output.stdout);
        assert_ne!(
            stdout, binary,
            "{arguments:?} prints the command's own help"
        );
        for named in [method, "not served"] {
            assert!(
                stdout.contains(named),
                "{arguments:?} names {named}: {stdout}"
            );
        }
    }
    for arguments in [
        &["new", "--help"][..],
        &["new", "demo", "--help"],
        &["-h", "new"],
    ] {
        let output = run(arguments);
        assert_eq!(output.status.code(), Some(0), "{arguments:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{arguments:?}: {output:?}");
        let stdout = text(&output.stdout);
        assert_ne!(stdout, binary, "{arguments:?} prints new's own help");
        assert!(
            stdout.contains("plasmid new <name>"),
            "{arguments:?}: {stdout}"
        );
    }
    let error = usage_error(&run(&["frobnicate", "--help"]));
    assert_eq!(error["argument"], "frobnicate");
    let error = usage_error(&run(&["add", "x", "y", "--help"]));
    assert_eq!(error["argument"], "y");
}

#[test]
fn unserved_commands_refuse_with_not_served() {
    for (command, positionals, method) in UNSERVED {
        let mut arguments = vec![command];
        arguments.extend_from_slice(positionals);
        assert_not_served(&run(&arguments), method);
    }
    let mut hostile = plasmid();
    hostile.env("PLASMOSOME_KERNEL", "../x").env_remove("HOME");
    assert_not_served(&run_with(hostile, &["add", "x"]), "plasmid.add");
    for arguments in [
        &["add", "x", "--kernel", "../x"][..],
        &["add", "x", "--frob"],
        &["--frob", "add", "x"],
        &["add", "x", "--deadline-ms", "0"],
        &["add", "x", "--json"],
    ] {
        assert_not_served(&run(arguments), "plasmid.add");
    }

    let output = run(&["add", "x", "--human"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = text(&output.stderr);
    assert!(stderr.ends_with("(client not_served)\n"), "{stderr:?}");
    assert!(
        serde_json::from_str::<serde_json::Value>(&stderr).is_err(),
        "{stderr:?}"
    );

    for (arguments, named) in [
        (&["add", "x", "--json", "--human"][..], "--human"),
        (&["add", "x", "--human", "--json"], "--json"),
        (&["add", "x", "--human", "--human"], "--human"),
        (&["add", "x", "--json", "--json"], "--json"),
    ] {
        let error = usage_error(&run(arguments));
        assert_eq!(error["argument"], named, "{arguments:?}");
    }
}

#[cfg(unix)]
#[test]
fn an_unserved_command_is_refused_before_its_arguments_are_read() {
    use std::os::unix::ffi::OsStrExt;
    let output = run(&[OsStr::new("add"), OsStr::from_bytes(b"x\xff")]);
    assert_not_served(&output, "plasmid.add");
}

#[test]
fn usage_errors_are_invalid_envelopes() {
    for (arguments, argument) in [
        (&["frobnicate"][..], Some("frobnicate")),
        (&[], None),
        (&["help"], Some("help")),
        (&["attach"], Some("attach")),
        (&["--frob"], Some("--frob")),
        (&["--json"], None),
    ] {
        let error = usage_error(&run(arguments));
        match argument {
            Some(argument) => assert_eq!(error["argument"], argument, "{arguments:?}"),
            None => assert!(error.get("argument").is_none(), "{arguments:?}: {error:?}"),
        }
    }
}

#[cfg(unix)]
#[test]
fn an_argument_that_is_not_utf8_is_a_usage_error_naming_its_lossy_form() {
    use std::os::unix::ffi::OsStrExt;
    let error = usage_error(&run(&[OsStr::from_bytes(b"fr\xffob")]));
    assert_eq!(
        error["argument"],
        format!("fr{}ob", char::REPLACEMENT_CHARACTER)
    );
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn a_refused_argument_round_trips_and_never_prints_a_control() {
    let controls: [u32; 5] = [0x1b, 0x9b, 0x7f, 0x202e, 0x0a];
    let backslash = char::from(0x5c_u8);
    let mut argument = format!("fr{}o{backslash}b", '"');
    argument.extend(controls.map(|code| char::from_u32(code).expect("a character")));
    let raw: [&[u8]; 4] = [&[0x1b], &[0x7f], &[0xc2, 0x9b], &[0xe2, 0x80, 0xae]];

    let output = run(&[argument.as_str()]);
    for bytes in raw {
        assert!(!contains(&output.stderr, bytes), "{bytes:x?}: {output:?}");
    }
    let error = usage_error(&output);
    assert_eq!(error["argument"], argument.as_str());

    let output = run(&[argument.as_str(), "--human"]);
    assert_eq!(output.status.code(), Some(64), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    for bytes in raw {
        assert!(!contains(&output.stderr, bytes), "{bytes:x?}: {output:?}");
    }
    let stderr = text(&output.stderr);
    for shown in ["<U+001B>", "<U+009B>", "<U+007F>", "<U+202E>", "<U+000A>"] {
        assert!(stderr.contains(shown), "{shown}: {stderr:?}");
    }
    assert!(
        stderr.contains(&format!("fr{}o{backslash}b<U+001B>", '"')),
        "{stderr:?}"
    );
    let lines: Vec<&str> = stderr.lines().collect();
    assert!(stderr.ends_with('\n'), "{stderr:?}");
    assert_eq!(lines.len(), 2, "{stderr:?}");
    assert_eq!(lines[1], "(client invalid_argument)", "{stderr:?}");
}

#[test]
fn help_and_version_to_a_closed_stdout_exit_1() {
    for arguments in [&["--help"][..], &["--version"], &["add", "--help"]] {
        let mut command = plasmid();
        command.stdout(closed_pipe());
        let output = run_with(command, arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.is_empty(), "{arguments:?}");
        assert!(!stderr.contains("panicked"), "{arguments:?}: {stderr}");
    }
}

#[test]
fn an_envelope_to_a_closed_stderr_still_exits_with_its_code() {
    for (arguments, code) in [(&["frobnicate"][..], 64), (&["add", "x"], 2)] {
        let mut command = plasmid();
        command.stderr(closed_pipe());
        let output = run_with(command, arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{arguments:?}: {output:?}"
        );
        assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
    }
}
