use std::ffi::OsStr;
use std::process::{Command, Output};

mod support;

use support::{entries, envelope, keys, text, usage_error};

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
    assert_eq!(
        keys(&error),
        ["code", "domain", "message", "method"],
        "{output:?}"
    );
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
        &["--version", "--", "x"],
        &["--kernel", "--version"],
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

fn command_help(command: &str) -> String {
    let output = run(&[command, "--help"]);
    assert_eq!(output.status.code(), Some(0), "{command}: {output:?}");
    text(&output.stdout)
}

#[test]
fn help_is_read_from_a_value_and_up_to_a_bare_flag_it_does_not_know() {
    let binary = binary_help();
    for (arguments, expected) in [
        (&["--kernel", "--help"][..], binary.clone()),
        (&["--socket", "-h"], binary.clone()),
        (&["--frob", "add", "x", "--help"], binary),
        (&["--frob=x", "add", "x", "--help"], command_help("add")),
        (
            &["add", "my-thing", "--cell", "c1", "--help"],
            command_help("add"),
        ),
        (&["list", "--cell", "c1", "--help"], command_help("list")),
    ] {
        let output = run(arguments);
        assert_eq!(output.status.code(), Some(0), "{arguments:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{arguments:?}: {output:?}");
        assert_eq!(text(&output.stdout), expected, "{arguments:?}");
    }
}

#[test]
fn help_after_dashes_is_not_read() {
    for flag in ["--help", "-h"] {
        assert_not_served(&run(&["add", "x", "--", flag]), "plasmid.add");
        let error = usage_error(&run(&["--", flag]));
        assert_eq!(error["argument"], "--", "{flag}");
    }
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
        &["--frob=x", "add", "x"],
        &["add", "x", "--deadline-ms", "0"],
        &["add", "x", "--json"],
        &["--deadline-ms", "5", "add", "x"],
        &["--socket", "/x", "add", "x"],
        &["add", "x", "--kernel", "a", "--kernel", "b"],
        &["add", "x", "--kernel"],
    ] {
        assert_not_served(&run(arguments), "plasmid.add");
    }

    let output = run(&["add", "x", "--human"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        text(&output.stderr),
        "plasmid add is not served by this build\n(client not_served)\n"
    );

    for (arguments, named) in [
        (&["add", "x", "--json", "--human"][..], "--human"),
        (&["add", "x", "--human", "--json"], "--json"),
        (&["add", "x", "--human", "--human"], "--human"),
        (&["add", "x", "--json", "--json"], "--json"),
        (&["add", "x", "--json", "--human", "--json"], "--human"),
        (&["add", "x", "--human", "--json", "--human"], "--json"),
        (&["add", "x", "--json", "--json", "--human"], "--json"),
        (&["--frob", "add", "x"], "--frob"),
        (&["--frob", "list"], "--frob"),
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

#[test]
fn the_full_parse_names_the_first_fault_on_the_line() {
    for (arguments, argument) in [
        (&["frobnicate", "--", "x"][..], "frobnicate"),
        (&["--", "frobnicate"], "--"),
        (&["frobnicate", "--frob"], "frobnicate"),
        (&["--frob", "frobnicate"], "--frob"),
        (&["frobnicate", "--json", "--json"], "frobnicate"),
        (&["--json", "--json", "frobnicate"], "--json"),
        (&["frobnicate", "--json", "--human"], "frobnicate"),
        (&["--json", "--human", "frobnicate"], "--human"),
        (&["frobnicate", "--kernel"], "frobnicate"),
        (&["--kernel", "--", "frobnicate"], "--kernel"),
        (&["x", "--kernel", "--", "y"], "x"),
        (&["--kernel", "a", "--kernel", "b"], "--kernel"),
        (&["--json", "--json"], "--json"),
        (&["--socket"], "--socket"),
        (&["--kernel", "--"], "--kernel"),
        (&["--json", "--human"], "--human"),
        (&["--human", "--json"], "--json"),
        (&["--json", "--json", "--frob"], "--json"),
        (&["--frob", "--json", "--json"], "--frob"),
        (&["--json", "--human", "--frob"], "--human"),
        (&["--frob", "--json", "--human"], "--frob"),
        (&["--json", "--human", "--json"], "--human"),
        (&["--human", "--json", "--human"], "--json"),
        (&["--json", "--json", "--human"], "--json"),
        (
            &["--kernel", "a", "--json", "--human", "--kernel", "b"],
            "--human",
        ),
        (
            &["--kernel", "a", "--kernel", "b", "--json", "--human"],
            "--kernel",
        ),
        (&["--json", "--human", "--kernel"], "--human"),
        (&["--json", "--json", "--", "x"], "--json"),
        (&["--", "--json", "--json"], "--"),
        (&["--json", "--human", "--"], "--human"),
        (&["--", "--json", "--human"], "--"),
        (&["list", "a", "--help"], "a"),
    ] {
        let error = usage_error(&run(arguments));
        assert_eq!(error["argument"], argument, "{arguments:?}");
    }
}

#[test]
fn a_form_flag_in_a_value_position_is_the_value() {
    assert_not_served(&run(&["add", "x", "--kernel", "--human"]), "plasmid.add");
    let error = usage_error(&run(&["frobnicate", "--kernel", "--human"]));
    assert_eq!(error["argument"], "frobnicate");
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

#[cfg(unix)]
#[test]
fn an_argument_that_is_not_utf8_is_a_fault_where_it_stands() {
    use std::os::unix::ffi::OsStrExt;
    let rows: [(&[&[u8]], &[u8]); 11] = [
        (&[b"frobnicate", b"a\xff"], b"frobnicate"),
        (&[b"a\xff", b"frobnicate"], b"a\xff"),
        (&[b"--frob", b"\xff"], b"--frob"),
        (&[b"\xff", b"--frob"], b"\xff"),
        (&[b"--frob", b"--", b"\xff"], b"--frob"),
        (&[b"--", b"\xff"], b"--"),
        (&[b"\xff", b"--"], b"\xff"),
        (&[b"--kernel", b"\xff", b"--frob"], b"\xff"),
        (&[b"new", b"demo", b"--", b"\xff"], b"--"),
        (&[b"new", b"Demo", b"x\xff"], b"Demo"),
        (&[b"new", b"x\xff", b"Demo"], b"x\xff"),
    ];
    for (arguments, argument) in rows {
        let arguments: Vec<&OsStr> = arguments
            .iter()
            .map(|argument| OsStr::from_bytes(argument))
            .collect();
        let error = usage_error(&run(&arguments));
        assert_eq!(
            error["argument"],
            String::from_utf8_lossy(argument).as_ref(),
            "{arguments:?}"
        );
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn terminal_controls() -> Vec<char> {
    [0x01..=0x1f, 0x7f..=0x9f, 0x202a..=0x202e, 0x2066..=0x2069]
        .into_iter()
        .flatten()
        .map(|code| char::from_u32(code).expect("a character"))
        .collect()
}

fn assert_no_raw_control(stderr: &[u8], lines: usize) {
    let shown = String::from_utf8_lossy(stderr);
    let pieces: Vec<&[u8]> = stderr.split(|byte| *byte == b'\n').collect();
    assert_eq!(pieces.len(), lines + 1, "{lines} lines: {shown:?}");
    assert!(pieces[lines].is_empty(), "a final newline: {shown:?}");
    for control in terminal_controls() {
        let mut encoded = [0; 4];
        let bytes = control.encode_utf8(&mut encoded).as_bytes();
        for piece in &pieces {
            assert!(
                !contains(piece, bytes),
                "U+{:04X} is raw: {shown:?}",
                u32::from(control)
            );
        }
    }
}

#[test]
fn a_refused_argument_round_trips_and_never_prints_a_control() {
    let controls = terminal_controls();
    assert_eq!(controls.len(), 73);
    let backslash = char::from(0x5c_u8);
    let neighbours: String = [
        0x20, 0x7e, 0xa0, 0x2028, 0x2029, 0x202f, 0x2065, 0x206a, 0xfeff,
    ]
    .into_iter()
    .map(|code| char::from_u32(code).expect("a character"))
    .collect();
    let prefix = format!("fr{}o{backslash}b", '"');
    let argument = format!("{prefix}{}{neighbours}", String::from_iter(&controls));

    let output = run(&[argument.as_str()]);
    assert_no_raw_control(&output.stderr, 1);
    let stderr = text(&output.stderr);
    let mut escaped = format!("fr{backslash}{}o{backslash}{backslash}b", '"');
    for control in &controls {
        let escape = format!("{backslash}u{:04x}", u32::from(*control));
        assert!(stderr.contains(&escape), "{escape}: {stderr:?}");
        escaped.push_str(&escape);
    }
    escaped.push_str(&neighbours);
    assert!(stderr.contains(&escaped), "{stderr:?}");
    let error = usage_error(&output);
    assert_eq!(error["argument"], argument.as_str());

    let output = run(&[argument.as_str(), "--human"]);
    assert_eq!(output.status.code(), Some(64), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_no_raw_control(&output.stderr, 2);
    let stderr = text(&output.stderr);
    let mut shown = prefix;
    for control in &controls {
        let show = format!("<U+{:04X}>", u32::from(*control));
        assert!(stderr.contains(&show), "{show}: {stderr:?}");
        shown.push_str(&show);
    }
    shown.push_str(&neighbours);
    assert!(stderr.contains(&shown), "{stderr:?}");
    assert!(
        stderr.ends_with("\n(client invalid_argument)\n"),
        "{stderr:?}"
    );
}
