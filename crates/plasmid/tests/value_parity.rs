#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::process::Command;

use plasmosome_cli::{COMMANDS, Context, Env, SystemClock, handle};
use serde_json::{Map, Value};

const RULES: &str = "crates/plasmid/src/values.rs and crates/plasmosome-cli/src/cli.rs";

const ERROR_KEYS: [&str; 4] = ["domain", "code", "message", "argument"];

#[derive(Clone, Copy)]
enum Form {
    Json,
    Human,
}

enum Answer {
    Usage {
        argument: Option<String>,
        form: Form,
    },
    Help,
}

impl Answer {
    fn describe(&self) -> String {
        match self {
            Answer::Help => "exit 0 with help on stdout".to_string(),
            Answer::Usage { argument, form } => {
                let named = match argument {
                    Some(argument) => format!("naming {argument:?}"),
                    None => "naming no argument".to_string(),
                };
                let form = match form {
                    Form::Json => "in JSON",
                    Form::Human => "in the human form",
                };
                format!("exit 64 {named} {form}")
            }
        }
    }
}

struct Row {
    name: String,
    line: Vec<OsString>,
    answer: Answer,
}

struct Run {
    exit: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct Unread;

impl Env for Unread {
    fn home(&self) -> Option<OsString> {
        panic!("HOME was read")
    }

    fn kernel(&self) -> Option<OsString> {
        panic!("PLASMOSOME_KERNEL was read")
    }
}

fn plasmid(line: &[OsString]) -> Run {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let output = Command::new(env!("CARGO_BIN_EXE_plasmid"))
        .env_remove("PLASMOSOME_KERNEL")
        .current_dir(scratch.path())
        .args(line)
        .output()
        .expect("the plasmid binary runs");
    Run {
        exit: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn plasmosome(line: &[OsString]) -> Run {
    let context = Context {
        version: "0.0.0",
        commands: COMMANDS,
        trusted_uid: 0,
        clock: &SystemClock,
        env: &Unread,
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = handle(line, &context, &mut stdout, &mut stderr);
    Run {
        exit: Some(i32::from(exit.code())),
        stdout,
        stderr,
    }
}

fn is_terminal_control(character: char) -> bool {
    matches!(
        u32::from(character),
        0x00..=0x1f | 0x7f..=0x9f | 0x202a..=0x202e | 0x2066..=0x2069
    )
}

fn shown(text: &str) -> String {
    text.chars()
        .map(|character| {
            if is_terminal_control(character) {
                format!("<U+{:04X}>", u32::from(character))
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn check(holds: bool, wanted: &str) -> Result<(), String> {
    if holds {
        Ok(())
    } else {
        Err(format!("wanted {wanted}"))
    }
}

fn lines(stderr: &str, count: usize) -> Result<Vec<&str>, String> {
    let body = stderr
        .strip_suffix('\n')
        .ok_or("wanted stderr to end in a newline")?;
    let lines: Vec<&str> = body.split('\n').collect();
    check(lines.len() == count, &format!("{count} lines on stderr"))?;
    check(
        lines
            .iter()
            .all(|line| !line.chars().any(is_terminal_control)),
        "no raw terminal control on stderr",
    )?;
    Ok(lines)
}

fn sorted_keys(object: &Map<String, Value>) -> Vec<&str> {
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

fn json_usage(stderr: &str, argument: Option<&str>) -> Result<(), String> {
    let line = lines(stderr, 1)?[0];
    let value: Value = serde_json::from_str(line)
        .map_err(|error| format!("wanted one JSON envelope on stderr: {error}"))?;
    let envelope = value.as_object().ok_or("wanted an envelope object")?;
    check(
        sorted_keys(envelope) == ["command", "error", "schema"],
        "the keys command, error and schema",
    )?;
    check(envelope.get("schema") == Some(&Value::from(1)), "schema 1")?;
    check(
        envelope.get("command") == Some(&Value::from("invalid")),
        "command invalid",
    )?;
    let error = envelope
        .get("error")
        .and_then(Value::as_object)
        .ok_or("wanted an error object")?;
    check(
        error.get("domain") == Some(&Value::from("client")),
        "domain client",
    )?;
    check(
        error.get("code") == Some(&Value::from("invalid_argument")),
        "code invalid_argument",
    )?;
    check(
        error.keys().all(|key| ERROR_KEYS.contains(&key.as_str())),
        "no error key but domain, code, message and argument",
    )?;
    check(
        error.get("argument") == argument.map(Value::from).as_ref(),
        &format!("argument {argument:?}"),
    )
}

fn human_usage(stderr: &str, argument: Option<&str>) -> Result<(), String> {
    let lines = lines(stderr, 2)?;
    check(
        lines[1] == "(client invalid_argument)",
        "a second line (client invalid_argument)",
    )?;
    let Some(argument) = argument else {
        return Ok(());
    };
    let shown = shown(argument);
    check(
        lines[0].contains(&shown),
        &format!("a message showing {shown:?}"),
    )
}

fn judge(answer: &Answer, run: &Run) -> Result<(), String> {
    match answer {
        Answer::Help => {
            check(run.exit == Some(0), "exit 0")?;
            check(run.stderr.is_empty(), "an empty stderr")?;
            check(!run.stdout.is_empty(), "help on stdout")
        }
        Answer::Usage { argument, form } => {
            check(run.exit == Some(64), "exit 64")?;
            check(run.stdout.is_empty(), "an empty stdout")?;
            let stderr = std::str::from_utf8(&run.stderr)
                .map_err(|error| format!("wanted UTF-8 on stderr: {error}"))?;
            match form {
                Form::Json => json_usage(stderr, argument.as_deref()),
                Form::Human => human_usage(stderr, argument.as_deref()),
            }
        }
    }
}

fn verdict(answer: &Answer, run: &Run) -> Option<String> {
    judge(answer, run).err().map(|why| {
        format!(
            "{why}; got exit {:?}, {} bytes on stdout, stderr {:?}",
            run.exit,
            run.stdout.len(),
            String::from_utf8_lossy(&run.stderr)
        )
    })
}

fn character(code: u32) -> char {
    char::from_u32(code).expect("a character")
}

fn around(code: u32) -> String {
    format!("a{}b", character(code))
}

fn words(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}

fn usage(name: &str, line: Vec<OsString>, argument: Option<&str>, form: Form) -> Row {
    Row {
        name: name.to_string(),
        line,
        answer: Answer::Usage {
            argument: argument.map(str::to_string),
            form,
        },
    }
}

fn names(name: &str, arguments: &[&str], argument: &str) -> Row {
    usage(name, words(arguments), Some(argument), Form::Json)
}

fn names_nothing(name: &str, arguments: &[&str]) -> Row {
    usage(name, words(arguments), None, Form::Json)
}

fn shows(name: &str, arguments: &[&str], argument: &str) -> Row {
    usage(name, words(arguments), Some(argument), Form::Human)
}

fn bytes(name: &str, arguments: &[&[u8]], argument: &str) -> Row {
    let line = arguments
        .iter()
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect();
    usage(name, line, Some(argument), Form::Json)
}

fn helps(name: &str, arguments: &[&str]) -> Row {
    Row {
        name: name.to_string(),
        line: words(arguments),
        answer: Answer::Help,
    }
}

fn reported() -> Vec<Row> {
    vec![
        names("A1", &["--kernel", "../x", "frobnicate"], "../x"),
        names("A2", &["--socket", "rel", "frobnicate"], "rel"),
        names("A3", &["--kernel", "", "frobnicate"], ""),
        names("A4", &["--kernel", "../x"], "../x"),
        names("A5", &["--socket", "rel"], "rel"),
        names("A6", &["--deadline-ms", "0"], "0"),
        names("A7", &["--kernel", "../x", "--kernel", "a"], "../x"),
        names("A8", &["--kernel", "../x", "--frob"], "../x"),
        names("A9", &["--deadline-ms", "0", "--json", "--json"], "0"),
    ]
}

fn edges() -> Vec<Row> {
    let owned = |values: &[&str]| values.iter().map(|value| value.to_string()).collect();
    let mut kernel_refused: Vec<String> = owned(&[".", "..", "a/b"]);
    kernel_refused.push(format!("a{}b", char::from(0x5c_u8)));
    kernel_refused.extend([0x1b, 0x7f, 0x9b, 0x202e, 0x2066].map(around));
    let mut kernel_accepted: Vec<String> = owned(&["work", "Work_1", "...", "-x"]);
    kernel_accepted.push(around(0x2028));
    let socket_refused = owned(&["rel", "", "./x", "/", "/a/", "/a//b", "/a/./b", "/a/../b"]);
    let mut socket_accepted: Vec<String> = owned(&["/a/b", "/a/.b", "/a/b c"]);
    socket_accepted.push(format!("/a{}b", character(0x1b)));
    socket_accepted.push(format!("/{}", "a".repeat(199)));
    let mut deadline_refused: Vec<String> =
        owned(&["0", "3600001", "00000001", "+5", "-5", "", "1e3"]);
    deadline_refused.push("1".repeat(40));
    let deadline_accepted = owned(&["1", "0000001", "3600000"]);
    let mut rows = Vec::new();
    for (flag, refused, accepted) in [
        ("--kernel", kernel_refused, kernel_accepted),
        ("--socket", socket_refused, socket_accepted),
        ("--deadline-ms", deadline_refused, deadline_accepted),
    ] {
        for value in &refused {
            rows.push(edge(flag, value, value));
        }
        for value in &accepted {
            rows.push(edge(flag, value, "frobnicate"));
        }
    }
    rows
}

fn edge(flag: &str, value: &str, argument: &str) -> Row {
    names(
        &format!("C {flag} {value:?}"),
        &[flag, value, "frobnicate"],
        argument,
    )
}

fn order() -> Vec<Row> {
    let replaced = char::REPLACEMENT_CHARACTER;
    vec![
        names("D1", &["--frob", "--kernel", "../x"], "--frob"),
        names("D2", &["--json", "--json", "--kernel", "../x"], "--json"),
        bytes(
            "D3",
            &[b"a\xff", b"--kernel", b"../x"],
            &format!("a{replaced}"),
        ),
        names("D4", &["frobnicate", "--kernel", "../x"], "frobnicate"),
        names("D5", &["--kernel", "a", "--kernel", "../x"], "--kernel"),
        names("D6", &["--kernel", "../x", "--json", "--json"], "../x"),
        bytes("D7", &[b"--kernel", b"../x", b"a\xff"], "../x"),
        names("D8", &["--kernel", "../x", "--", "x"], "../x"),
        names("D9", &["--kernel", "../x", "--"], "../x"),
        names("D10", &["--socket", "rel", "--kernel", "../x"], "rel"),
        names("D11", &["--deadline-ms", "0", "--socket", "rel"], "0"),
        names("D12", &["--socket", "--human"], "--human"),
        names("D13", &["--deadline-ms", "--json"], "--json"),
        names_nothing("D14", &["--kernel", "--json"]),
        bytes(
            "D15",
            &[b"--kernel", b"\xff", b"frobnicate"],
            &replaced.to_string(),
        ),
    ]
}

fn human() -> Vec<Row> {
    let control = around(0x1b);
    vec![
        shows("E1", &["--human", "--kernel", "../x", "frobnicate"], "../x"),
        shows("E2", &["--kernel", &control, "--human"], &control),
    ]
}

fn first() -> Vec<Row> {
    vec![
        helps("F1", &["--kernel", "../x", "--help"]),
        helps("F2", &["--deadline-ms", "0", "-h"]),
        names(
            "F3",
            &["--kernel", "../x", "frobnicate", "--help"],
            "frobnicate",
        ),
        names("F4", &["--kernel", "../x", "--version"], "--version"),
        names("F5", &["--socket", "rel", "--version"], "--version"),
        names("F6", &["--kernel", "--", "--help"], "--kernel"),
    ]
}

#[test]
fn plasmid_and_plasmosome_name_the_same_argument_on_every_line() {
    let rows: Vec<Row> = [reported(), edges(), order(), human(), first()]
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(
        rows.len(),
        70,
        "the design's 69 lines and --kernel -- --help"
    );
    let mut differ = Vec::new();
    for row in &rows {
        let plasmid = verdict(&row.answer, &plasmid(&row.line));
        let plasmosome = verdict(&row.answer, &plasmosome(&row.line));
        if plasmid.is_none() && plasmosome.is_none() {
            continue;
        }
        let side = |verdict: Option<String>| verdict.unwrap_or_else(|| "as expected".to_string());
        differ.push(format!(
            "{} {:?}: expected {}\n  plasmid: {}\n  plasmosome: {}",
            row.name,
            row.line,
            row.answer.describe(),
            side(plasmid),
            side(plasmosome)
        ));
    }
    assert!(
        differ.is_empty(),
        "{} of {} lines differ. The value rules are in {RULES}.\n{}",
        differ.len(),
        rows.len(),
        differ.join("\n")
    );
}
