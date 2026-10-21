use std::ffi::OsString;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixListener;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_plasmosome");

const VERSION: &str = env!("CARGO_PKG_VERSION");

const EPIPE: i64 = 32;

const COMMANDS: [(&str, &str); 23] = [
    ("status", "plasmosome.status"),
    ("recovery", "plasmosome.recovery"),
    ("stop", "plasmosome.stop"),
    ("start", "plasmosome.start"),
    ("list", "plasmosome.list"),
    ("cell new", "cell.new"),
    ("germinate GENOME", "cell.new"),
    ("cell list", "cell.list"),
    ("cell status", "cell.status"),
    ("cell kill", "cell.kill"),
    ("cell exec", "cell.exec"),
    ("exec status", "exec.status"),
    ("exec output", "exec.output"),
    ("registry source add ALIAS", "source.add"),
    ("registry source list", "source.list"),
    ("registry source remove ALIAS", "source.remove"),
    ("registry search", "search"),
    ("registry show REF", "show"),
    ("registry inspect DIRECTORY", "inspect"),
    ("registry fetch REF", "fetch"),
    ("registry register DIRECTORY", "register"),
    ("registry yank REF", "yank"),
    ("registry import DIRECTORY", "import"),
];

fn plasmosome(args: &[&str]) -> Output {
    run(args.iter().map(OsString::from).collect(), |_| {})
}

fn run(args: Vec<OsString>, configure: impl FnOnce(&mut Command)) -> Output {
    let mut command = Command::new(BINARY);
    command.args(args).stdin(Stdio::null());
    configure(&mut command);
    command.output().expect("the plasmosome binary runs")
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .unwrap_or_else(|| panic!("plasmosome ended without an exit code: {output:?}"))
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("the output is UTF-8")
}

fn envelope(output: &Output) -> Value {
    assert!(
        output.stdout.is_empty(),
        "a failure wrote to stdout: {output:?}"
    );
    let stderr = text(&output.stderr);
    assert!(
        stderr.ends_with('\n') && stderr.matches('\n').count() == 1,
        "stderr is not one line: {stderr:?}"
    );
    serde_json::from_str(stderr).expect("stderr is one JSON envelope")
}

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

fn assert_usage(output: &Output, argument: Option<&str>) {
    assert_eq!(code(output), 64, "{output:?}");
    let envelope = envelope(output);
    assert_eq!(keys(&envelope), ["command", "error", "schema"]);
    assert_eq!(envelope["schema"], 1);
    assert_eq!(envelope["command"], "invalid");
    assert_eq!(envelope["error"]["domain"], "client");
    assert_eq!(envelope["error"]["code"], "invalid_argument");
    assert_eq!(
        envelope["error"]
            .get("argument")
            .map(|value| value.as_str()),
        argument.map(Some),
        "{envelope}"
    );
}

fn live_instance() -> (TempDir, UnixListener) {
    let home = tempfile::tempdir().expect("a test HOME");
    let instance = home.path().join(".plasmosome/instances/a");
    std::fs::create_dir_all(&instance).expect("the instance directory");
    let listener =
        UnixListener::bind(instance.join("control.uds")).expect("a live socket for instance a");
    listener
        .set_nonblocking(true)
        .expect("the listener does not block");
    (home, listener)
}

fn assert_never_reached(listener: &UnixListener, what: &str) {
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        other => panic!("{what} reached the instance's socket: {other:?}"),
    }
}

fn listed(help: &str, usage: &str) -> Vec<String> {
    let start = format!("  plasmosome {usage} ");
    help.lines()
        .filter(|line| line.starts_with(&start))
        .map(str::to_string)
        .collect()
}

#[test]
fn version_is_the_sole_argument() {
    let output = run(vec!["--version".into()], |command| {
        command.env("PLASMOSOME_KERNEL", "../x").env_remove("HOME");
    });
    assert_eq!(code(&output), 0, "{output:?}");
    assert_eq!(text(&output.stdout), format!("plasmosome {VERSION}\n"));
    assert!(output.stderr.is_empty(), "{output:?}");
    for line in [
        &["status", "--version"][..],
        &["--version", "--json"],
        &["--version", "--version"],
        &["cell", "list", "--version"],
        &["--version", "--"],
    ] {
        assert_usage(&plasmosome(line), Some("--version"));
    }
    let workload = plasmosome(&["cell", "exec", "--", "python3", "--version"]);
    assert_eq!(code(&workload), 2, "{workload:?}");
    assert_eq!(envelope(&workload)["error"]["code"], "not_served");
}

#[test]
fn help_comes_first_and_never_connects() {
    let (home, listener) = live_instance();
    let with_home = |args: &[&str]| {
        run(args.iter().map(OsString::from).collect(), |command| {
            command.env("HOME", home.path());
        })
    };

    let output = with_home(&["--help"]);
    assert_eq!(code(&output), 0, "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let help = text(&output.stdout);
    for (usage, method) in COMMANDS {
        let rows = listed(help, usage);
        assert_eq!(rows.len(), 1, "{usage} is not listed once in {help}");
        assert!(
            rows[0].ends_with(" not served"),
            "{usage} is not marked not served: {:?}",
            rows[0]
        );
        let verb = if usage.starts_with("registry ") {
            "none"
        } else {
            method
        };
        assert!(
            rows[0].split_whitespace().any(|word| word == verb),
            "{usage} does not name {verb}: {:?}",
            rows[0]
        );
    }
    for item in [
        "--kernel NAME",
        "--socket PATH",
        "--deadline-ms N",
        "--json",
        "--human",
        "-h, --help",
        "--version",
        "PLASMOSOME_KERNEL",
    ] {
        assert!(help.contains(item), "the help does not give {item}: {help}");
    }
    for exit in ["0", "1", "2", "3", "64", "70"] {
        assert!(
            help.lines()
                .any(|line| line.starts_with(&format!("  {exit} "))),
            "the help does not give exit {exit}: {help}"
        );
    }
    assert!(
        !help.contains("uid"),
        "the help names a way to the trusted UID: {help}"
    );
    assert_eq!(with_home(&["-h"]).stdout, output.stdout);
    let flagged = run(
        ["--help", "--kernel", "../x", "--json", "--human"]
            .iter()
            .map(OsString::from)
            .collect(),
        |command| {
            command
                .env("HOME", home.path())
                .env("PLASMOSOME_KERNEL", "../x");
        },
    );
    assert_eq!(code(&flagged), 0, "{flagged:?}");
    assert_eq!(flagged.stdout, output.stdout);

    let group = with_home(&["cell", "--help"]);
    assert_eq!(code(&group), 0, "{group:?}");
    let rows: Vec<&str> = text(&group.stdout)
        .lines()
        .filter(|line| line.starts_with("  plasmosome "))
        .collect();
    assert_eq!(rows.len(), 5, "{rows:?}");
    for usage in [
        "cell new",
        "cell list",
        "cell status",
        "cell kill",
        "cell exec",
    ] {
        assert_eq!(listed(text(&group.stdout), usage).len(), 1, "{rows:?}");
    }

    for line in [&["cell", "list", "--help"][..], &["--help", "cell", "list"]] {
        let command = with_home(line);
        assert_eq!(code(&command), 0, "{command:?}");
        let help = text(&command.stdout);
        assert!(help.contains("plasmosome cell list"), "{help}");
        assert!(help.contains("cell.list"), "{help}");
        assert!(help.contains("Not served in this build"), "{help}");
    }
    let germinate = with_home(&["germinate", "--help"]);
    assert_eq!(code(&germinate), 0, "{germinate:?}");
    assert!(text(&germinate.stdout).contains("plasmosome germinate GENOME"));
    for (line, usage) in [
        (
            &["registry", "search", "--source", "main", "--help"][..],
            "registry search",
        ),
        (
            &["registry", "fetch", "REF", "--output", "/x", "--help"],
            "registry fetch REF",
        ),
        (&["cell", "exec", "--cell", "c1", "--help"], "cell exec"),
        (
            &[
                "exec",
                "status",
                "--cell",
                "c1",
                "--exec-id",
                "e1",
                "--help",
            ],
            "exec status",
        ),
    ] {
        let command = with_home(line);
        assert_eq!(code(&command), 0, "{command:?}");
        assert!(
            text(&command.stdout).starts_with(&format!("Usage: plasmosome {usage} [FLAGS]\n")),
            "{command:?}"
        );
    }

    assert_usage(&with_home(&["frobnicate", "--help"]), Some("frobnicate"));
    assert_usage(&with_home(&["status", "extra", "--help"]), Some("extra"));

    assert_never_reached(&listener, "help");
}

#[test]
fn unserved_commands_never_connect() {
    let (home, listener) = live_instance();
    for line in [
        &["cell", "list"][..],
        &["cell", "list", "--kernel", "a"],
        &["registry", "search"],
    ] {
        let output = run(line.iter().map(OsString::from).collect(), |command| {
            command
                .env("HOME", home.path())
                .env("PLASMOSOME_KERNEL", "a");
        });
        assert_eq!(code(&output), 2, "{line:?}: {output:?}");
        assert_eq!(envelope(&output)["error"]["code"], "not_served", "{line:?}");
    }
    assert_never_reached(&listener, "an unserved command");
}

#[test]
fn unserved_commands_refuse_without_reading_more() {
    for (usage, command) in COMMANDS {
        let words: Vec<OsString> = usage
            .split(' ')
            .filter(|word| !word.chars().all(|c| c.is_ascii_uppercase()))
            .map(OsString::from)
            .collect();
        let output = run(words, |process| {
            process.env("PLASMOSOME_KERNEL", "../x").env_remove("HOME");
        });
        assert_eq!(code(&output), 2, "{usage}: {output:?}");
        let envelope = envelope(&output);
        assert_eq!(keys(&envelope), ["command", "error", "schema"], "{usage}");
        assert_eq!(envelope["schema"], 1);
        assert_eq!(envelope["command"], command, "{usage}");
        assert_eq!(envelope["error"]["domain"], "client");
        assert_eq!(envelope["error"]["code"], "not_served");
        assert!(envelope["error"]["message"].is_string());
        if usage.starts_with("registry ") {
            assert_eq!(
                keys(&envelope["error"]),
                ["code", "domain", "message"],
                "{usage}"
            );
        } else {
            assert_eq!(
                keys(&envelope["error"]),
                ["code", "domain", "message", "method"],
                "{usage}"
            );
            assert_eq!(envelope["error"]["method"], command, "{usage}");
        }
    }

    let output = plasmosome(&[
        "cell",
        "status",
        "--cell",
        "c1",
        "--kernel",
        "../x",
        "--frob",
        "--deadline-ms",
        "0",
    ]);
    assert_eq!(code(&output), 2, "{output:?}");
    assert_eq!(envelope(&output)["command"], "cell.status");

    let human = plasmosome(&["cell", "list", "--human"]);
    assert_eq!(code(&human), 2, "{human:?}");
    assert!(human.stdout.is_empty());
    let shown = text(&human.stderr);
    assert!(shown.ends_with("\n(client not_served)\n"), "{shown:?}");
    assert!(serde_json::from_str::<Value>(shown).is_err(), "{shown:?}");

    assert_usage(
        &plasmosome(&["cell", "list", "--json", "--human"]),
        Some("--human"),
    );
    assert_usage(
        &plasmosome(&["cell", "list", "--human", "--human"]),
        Some("--human"),
    );
    assert_usage(
        &plasmosome(&["cell", "list", "--json", "--json"]),
        Some("--json"),
    );
}

#[test]
fn usage_errors() {
    assert_usage(&plasmosome(&[]), None);
    assert_usage(&plasmosome(&["--json"]), None);
    assert_usage(&plasmosome(&["frobnicate"]), Some("frobnicate"));
    assert_usage(&plasmosome(&["frobnicate", "--", "x"]), Some("frobnicate"));
    assert_usage(&plasmosome(&["frobnicate", "--frob"]), Some("frobnicate"));
    assert_usage(
        &plasmosome(&["cell", "list", "--json", "--human", "--json"]),
        Some("--human"),
    );
    assert_usage(&plasmosome(&["--frob"]), Some("--frob"));
    assert_usage(&plasmosome(&["cell"]), Some("cell"));
    assert_usage(&plasmosome(&["--trusted-uid", "0"]), Some("--trusted-uid"));
    let lossy = format!("a{}b", char::REPLACEMENT_CHARACTER);
    assert_usage(
        &run(vec![OsString::from_vec(vec![b'a', 0xff, b'b'])], |_| {}),
        Some(&lossy),
    );

    let escape = char::from_u32(0x1b).expect("ESC is a code point");
    let word = format!("a{escape}[31m");
    let output = plasmosome(&[&word]);
    assert_usage(&output, Some(&word));
    let raw = text(&output.stderr);
    assert!(!raw.contains(escape), "a raw ESC reached stderr: {raw:?}");
    assert!(raw.contains(&format!("{}u001b", '\\')), "{raw:?}");
    let human = plasmosome(&[&word, "--human"]);
    assert_eq!(code(&human), 64, "{human:?}");
    assert!(human.stdout.is_empty());
    let shown = text(&human.stderr);
    assert!(
        !shown.contains(escape),
        "a raw ESC reached stderr: {shown:?}"
    );
    assert!(shown.contains("<U+001B>"), "{shown:?}");
    assert!(
        shown.ends_with("\n(client invalid_argument)\n"),
        "{shown:?}"
    );
}

#[test]
fn a_closed_stdout_is_local_io() {
    for (flag, command) in [("--help", "--help"), ("--version", "--version")] {
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let output = run(vec![flag.into()], |process| {
            process.stdout(writer);
        });
        assert_eq!(code(&output), 1, "{output:?}");
        let envelope = envelope(&output);
        assert_eq!(envelope["command"], command);
        assert_eq!(envelope["error"]["code"], "local_io");
        assert_eq!(envelope["error"]["errno"], EPIPE);
    }
}
