use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use plasmosome_cli::json::{Json, write_compact};
use plasmosome_cli::{COMMANDS, ClientCode, Command, CommandArgs, Flag, FlagKind, Reading, read};

const CELL: Flag = Flag {
    name: "--cell",
    key: "cell",
    kind: FlagKind::Text,
};

const NOW: Flag = Flag {
    name: "--now",
    key: "now",
    kind: FlagKind::Bare,
};

const MOCK: Flag = Flag {
    name: "--mock",
    key: "mock",
    kind: FlagKind::Mock,
};

const LEASE: Flag = Flag {
    name: "--lease-ms",
    key: "lease_ms",
    kind: FlagKind::Integer,
};

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

fn table() -> Vec<Command> {
    COMMANDS
        .iter()
        .map(|command| {
            let (flags, takes_argv): (&'static [Flag], bool) = match command.words {
                ["status"] => (&[], false),
                ["germinate"] => (&[CELL, NOW, MOCK, LEASE], false),
                ["cell", "status"] => (&[CELL], false),
                ["cell", "exec"] => (&[CELL], true),
                _ => return *command,
            };
            Command {
                served: true,
                flags,
                takes_argv,
                ..*command
            }
        })
        .collect()
}

fn line(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

fn parsed(words: &[&str]) -> CommandArgs {
    match read(&line(words), &table()) {
        Reading::Run(invocation) => invocation.args,
        other => panic!("{words:?} read as {other:?}"),
    }
}

fn sent(args: &CommandArgs, flag: &Flag) -> Option<String> {
    args.flag(flag).map(|param| {
        assert_eq!(param.key, flag.key);
        write_compact(&param.value)
    })
}

fn named(line: Vec<OsString>) -> Option<String> {
    let reading = read(&line, &table());
    let Reading::Refuse { command, error, .. } = &reading else {
        panic!("{line:?} read as {reading:?}");
    };
    assert_eq!(
        (command.as_str(), error.code()),
        ("invalid", ClientCode::InvalidArgument),
        "{line:?} read as {reading:?}"
    );
    error
        .fields()
        .iter()
        .find(|(key, _)| key == "argument")
        .map(|(_, value)| match value {
            Json::String(text) => text.clone(),
            other => panic!("`argument` is not a string: {other:?}"),
        })
}

fn help(words: &[&str]) -> String {
    match read(&line(words), &table()) {
        Reading::Help { text, .. } => text,
        other => panic!("{words:?} read as {other:?}"),
    }
}

#[test]
fn a_served_commands_own_flags_are_read_with_its_words() {
    let args = parsed(&[
        "germinate",
        "--cell",
        "cell-1",
        "genome-a",
        "--now",
        "--mock",
        "--lease-ms",
        "18446744073709551616",
    ]);
    let genome = args
        .positional("GENOME", "genome")
        .expect("GENOME is given");
    assert_eq!(genome.key, "genome");
    assert_eq!(write_compact(&genome.value), "\"genome-a\"");
    assert_eq!(sent(&args, &CELL).as_deref(), Some("\"cell-1\""));
    assert_eq!(sent(&args, &NOW).as_deref(), Some("true"));
    assert_eq!(sent(&args, &MOCK).as_deref(), Some("\"simulate\""));
    assert_eq!(sent(&args, &LEASE).as_deref(), Some("18446744073709551616"));
    assert_eq!(args.argv("argv"), None);

    let args = parsed(&["germinate", "genome-a", "--mock=capture", "--lease-ms", "0"]);
    assert_eq!(sent(&args, &MOCK).as_deref(), Some("\"capture\""));
    assert_eq!(sent(&args, &LEASE).as_deref(), Some("0"));
    assert_eq!(sent(&args, &NOW), None);
    assert_eq!(sent(&args, &CELL), None);
    assert_eq!(
        sent(&parsed(&["germinate", "g", "--mock="]), &MOCK).as_deref(),
        Some("\"\"")
    );
    assert_eq!(parsed(&["status"]), CommandArgs::default());
}

#[test]
fn a_flags_value_is_never_read_as_a_word() {
    for (words, cell, genome) in [
        (
            &["--cell", "status", "germinate", "g"][..],
            "\"status\"",
            "\"g\"",
        ),
        (
            &["germinate", "--cell", "germinate", "g"],
            "\"germinate\"",
            "\"g\"",
        ),
        (&["germinate", "--cell", "--now", "g"], "\"--now\"", "\"g\""),
    ] {
        let args = parsed(words);
        assert_eq!(sent(&args, &CELL).as_deref(), Some(cell), "{words:?}");
        let given = args
            .positional("GENOME", "genome")
            .map(|param| write_compact(&param.value));
        assert_eq!(given.as_deref(), Some(genome), "{words:?}");
    }
    let args = parsed(&["cell", "--cell", "status", "status"]);
    assert_eq!(sent(&args, &CELL).as_deref(), Some("\"status\""));
}

#[test]
fn the_argv_after_dashes_is_passed_unchanged() {
    let argv = |words: &[&str]| {
        parsed(words)
            .argv("argv")
            .map(|param| write_compact(&param.value))
    };
    assert_eq!(
        argv(&[
            "cell", "exec", "--cell", "c", "--", "run", "--frob", "--", "x"
        ])
        .as_deref(),
        Some("[\"run\",\"--frob\",\"--\",\"x\"]")
    );
    assert_eq!(argv(&["cell", "exec", "--"]).as_deref(), Some("[]"));
    assert_eq!(argv(&["cell", "exec"]), None);
    let mut bad = line(&["cell", "exec", "--", "run"]);
    bad.push(OsString::from_vec(vec![b'a', 0xff]));
    assert_eq!(
        named(bad),
        Some(format!("a{}", char::REPLACEMENT_CHARACTER))
    );
}

#[test]
fn each_fault_in_the_own_flags_names_its_argument() {
    for (words, argument) in [
        (
            &["germinate", "g", "--cell", "a", "--cell", "b"][..],
            "--cell",
        ),
        (&["germinate", "g", "--now", "--now"], "--now"),
        (
            &["germinate", "g", "--mock", "--mock=capture"],
            "--mock=capture",
        ),
        (&["germinate", "g", "--mock=a", "--mock"], "--mock"),
        (&["germinate", "g", "--cell"], "--cell"),
        (&["germinate", "g", "--lease-ms"], "--lease-ms"),
        (&["germinate", "g", "--frob"], "--frob"),
        (&["germinate", "g", "--cell=x"], "--cell=x"),
        (&["germinate", "g", "--lease-ms=5"], "--lease-ms=5"),
        (&["germinate", "g", "--now=true"], "--now=true"),
        (&["germinate", "g", "--mockery"], "--mockery"),
        (&["germinate", "g", "--mock", "capture"], "capture"),
        (&["germinate", "g", "extra"], "extra"),
        (&["germinate", "g", "--lease-ms", "007"], "007"),
        (&["germinate", "g", "--lease-ms", "-1"], "-1"),
        (&["germinate", "g", "--lease-ms", "1.5"], "1.5"),
        (&["germinate", "g", "--lease-ms", "1e3"], "1e3"),
        (&["germinate", "g", "--lease-ms", ""], ""),
        (&["germinate", "g", "--", "x"], "--"),
        (&["germinate", "g", "--"], "--"),
        (&["cell", "status", "--cell", "c", "--", "x"], "--"),
        (&["status", "--cell", "c"], "--cell"),
        (&["cell", "status", "--now"], "--now"),
        (&["cell", "exec", "--mock=a"], "--mock=a"),
    ] {
        assert_eq!(named(line(words)).as_deref(), Some(argument), "{words:?}");
    }
}

#[test]
fn a_missing_positional_names_no_argument() {
    for words in [
        &["germinate"][..],
        &["germinate", "--cell", "c"],
        &["germinate", "--now", "--json"],
    ] {
        assert_eq!(named(line(words)), None, "{words:?}");
    }
}

#[test]
fn the_leftmost_fault_is_named_first() {
    for (words, argument) in [
        (&["germinate", "g", "--frob", "--cell"][..], "--frob"),
        (
            &["germinate", "g", "--cell", "a", "extra", "--frob"],
            "extra",
        ),
        (
            &["germinate", "--lease-ms", "07", "g", "--now", "--now"],
            "07",
        ),
        (
            &["germinate", "--now", "--now", "--lease-ms", "07"],
            "--now",
        ),
        (&["germinate", "--cell=x", "--", "y"], "--cell=x"),
    ] {
        assert_eq!(named(line(words)).as_deref(), Some(argument), "{words:?}");
    }
}

#[test]
fn an_unserved_command_reads_none_of_the_flags() {
    for words in [
        &["cell", "list", "--cell", "c"][..],
        &["cell", "list", "--cell"],
        &["cell", "list", "--frob", "--lease-ms", "07"],
    ] {
        let reading = read(&line(words), &table());
        let Reading::Refuse { error, .. } = &reading else {
            panic!("{words:?} read as {reading:?}");
        };
        assert_eq!(error.code(), ClientCode::NotServed, "{words:?}");
    }
}

#[test]
fn help_reads_the_words_past_an_own_flags_value() {
    let cell_status = help(&["cell", "status", "--help"]);
    assert!(cell_status.starts_with("Usage: plasmosome cell status [FLAGS]\n"));
    for words in [
        &["cell", "status", "--cell", "cell-1", "--help"][..],
        &["cell", "--cell", "cell-1", "status", "--help"],
        &["--cell", "status", "cell", "status", "--help"],
    ] {
        assert_eq!(help(words), cell_status, "{words:?}");
    }
    let unserved: Vec<Command> = COMMANDS.to_vec();
    let reading = read(
        &line(&["cell", "--cell", "cell-1", "status", "--help"]),
        &unserved,
    );
    let Reading::Help { text, .. } = reading else {
        panic!("help is help");
    };
    assert!(text.starts_with("Commands under plasmosome cell"), "{text}");
}

#[test]
fn help_lists_a_commands_own_flags() {
    let germinate = help(&["germinate", "--help"]);
    assert!(
        germinate.contains("\nIts own flags: --cell VALUE, --now, --mock[=MODE], --lease-ms N.\n"),
        "{germinate}"
    );
    assert!(germinate.starts_with("Usage: plasmosome germinate GENOME [FLAGS]\n"));
    assert!(!germinate.contains("argv"), "{germinate}");
    let exec = help(&["cell", "exec", "--help"]);
    assert!(
        exec.contains("\nIts own flags: --cell VALUE.\nEverything after -- is the workload's argv, passed unchanged.\n"),
        "{exec}"
    );
    assert_eq!(
        help(&["status", "--help"]),
        "Usage: plasmosome status [FLAGS]\n\nSends the verb plasmosome.status, which changes no \
         state.\nServed in this build.\n\nRun plasmosome --help for the flags, PLASMOSOME_KERNEL \
         and the exit codes.\n"
    );
}

fn clashes(commands: &[Command]) -> Vec<String> {
    let flags: Vec<Flag> = commands
        .iter()
        .flat_map(|command| command.flags.iter().copied())
        .collect();
    let mut found = Vec::new();
    for (index, flag) in flags.iter().enumerate() {
        if !flag.name.starts_with("--") || GLOBAL_FLAGS.contains(&flag.name) {
            found.push(format!(
                "{} is not a name a command's own flag may take",
                flag.name
            ));
        }
        for other in &flags[..index] {
            if other.name == flag.name && other != flag {
                found.push(format!("{} is both {other:?} and {flag:?}", flag.name));
            }
        }
    }
    found
}

#[test]
fn each_flag_name_has_one_kind_and_key_across_the_table() {
    assert_eq!(clashes(COMMANDS), Vec::<String>::new());
    assert_eq!(clashes(&table()), Vec::<String>::new());
    let clashing: Vec<Command> = table()
        .into_iter()
        .map(|command| match command.words {
            ["cell", "status"] => Command {
                flags: &[Flag {
                    kind: FlagKind::Integer,
                    ..CELL
                }],
                ..command
            },
            ["cell", "exec"] => Command {
                flags: &[Flag {
                    name: "--kernel",
                    ..CELL
                }],
                ..command
            },
            _ => command,
        })
        .collect();
    assert_eq!(clashes(&clashing).len(), 2, "{:?}", clashes(&clashing));
}
