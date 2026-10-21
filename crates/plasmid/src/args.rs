use std::ffi::{OsStr, OsString};

use crate::envelope::{Form, Refusal};
use crate::help;

const SHORT_HELP: &str = "-h";

pub(crate) const HELP: &str = "--help";

pub(crate) const VERSION: &str = "--version";

const JSON: &str = "--json";

const HUMAN: &str = "--human";

const VALUE_FLAGS: [&str; 3] = ["--kernel", "--socket", "--deadline-ms"];

const FLAGS: [&str; 5] = ["--kernel", "--socket", "--deadline-ms", JSON, HUMAN];

const KNOWN_FLAGS: [&str; 8] = [
    "--kernel",
    "--socket",
    "--deadline-ms",
    JSON,
    HUMAN,
    SHORT_HELP,
    HELP,
    VERSION,
];

const NAME_LIMIT: usize = 64;

#[derive(Clone, Copy)]
pub(crate) enum Serving {
    Local,
    NotServed { method: &'static str },
}

pub(crate) struct Command {
    pub(crate) word: &'static str,
    pub(crate) positionals: usize,
    pub(crate) serving: Serving,
}

impl Command {
    pub(crate) fn usage(&self) -> String {
        let name = if self.positionals == 1 { " <name>" } else { "" };
        format!("{}{name}", self.word)
    }
}

pub(crate) const COMMANDS: [Command; 6] = [
    Command {
        word: "new",
        positionals: 1,
        serving: Serving::Local,
    },
    Command {
        word: "list",
        positionals: 0,
        serving: Serving::NotServed {
            method: "plasmid.list",
        },
    },
    Command {
        word: "add",
        positionals: 1,
        serving: Serving::NotServed {
            method: "plasmid.add",
        },
    },
    Command {
        word: "remove",
        positionals: 1,
        serving: Serving::NotServed {
            method: "plasmid.remove",
        },
    },
    Command {
        word: "reload",
        positionals: 1,
        serving: Serving::NotServed {
            method: "plasmid.reload",
        },
    },
    Command {
        word: "renew",
        positionals: 1,
        serving: Serving::NotServed {
            method: "plasmid.renew",
        },
    },
];

pub(crate) enum Reading<'a> {
    Help(String, Form),
    Version,
    Refuse(Refusal),
    New(&'a str),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Flag,
    Value,
}

struct Line<'a> {
    head: Vec<(Kind, &'a OsStr)>,
    dashes: bool,
}

pub(crate) fn read(arguments: &[OsString]) -> Reading<'_> {
    let line = Line::new(arguments);
    if line.anywhere(SHORT_HELP) || line.anywhere(HELP) {
        return line.help();
    }
    if arguments.len() == 1 && arguments[0] == VERSION {
        return Reading::Version;
    }
    if line.anywhere(VERSION) {
        return line.usage(
            format!("{VERSION} is read only as the sole argument"),
            Some(OsStr::new(VERSION)),
        );
    }
    if let Some((command, method)) = line.command_words().first().and_then(|word| unserved(word)) {
        return line.not_served(command, method);
    }
    line.parse()
}

fn command(word: &OsStr) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| word == command.word)
}

fn served(word: &OsStr) -> Option<&'static Command> {
    command(word).filter(|command| matches!(command.serving, Serving::Local))
}

fn unserved(word: &OsStr) -> Option<(&'static Command, &'static str)> {
    let command = command(word)?;
    match command.serving {
        Serving::NotServed { method } => Some((command, method)),
        Serving::Local => None,
    }
}

impl<'a> Line<'a> {
    fn new(arguments: &'a [OsString]) -> Line<'a> {
        let split = arguments
            .iter()
            .position(|argument| argument == "--")
            .unwrap_or(arguments.len());
        let mut head = Vec::with_capacity(split);
        let mut wants_value = false;
        for argument in &arguments[..split] {
            let kind = if wants_value {
                Kind::Value
            } else if argument.as_encoded_bytes().starts_with(b"-") {
                Kind::Flag
            } else {
                Kind::Word
            };
            wants_value = kind == Kind::Flag && VALUE_FLAGS.iter().any(|flag| argument == *flag);
            head.push((kind, argument.as_os_str()));
        }
        Line {
            head,
            dashes: split < arguments.len(),
        }
    }

    fn anywhere(&self, flag: &str) -> bool {
        self.head.iter().any(|(_, argument)| *argument == flag)
    }

    fn flags(&self) -> impl Iterator<Item = &'a OsStr> + '_ {
        self.head
            .iter()
            .filter(|(kind, _)| *kind == Kind::Flag)
            .map(|(_, flag)| *flag)
    }

    fn command_words(&self) -> Vec<&'a OsStr> {
        self.head
            .iter()
            .take_while(|(kind, argument)| {
                *kind != Kind::Flag
                    || KNOWN_FLAGS.iter().any(|known| *argument == *known)
                    || holds_its_value(argument)
            })
            .filter(|(kind, _)| *kind == Kind::Word)
            .map(|(_, word)| *word)
            .collect()
    }

    fn count(&self, flag: &str) -> usize {
        self.flags().filter(|argument| *argument == flag).count()
    }

    fn form(&self) -> Form {
        if self.count(HUMAN) == 1 && self.count(JSON) == 0 {
            Form::Human
        } else {
            Form::Json
        }
    }

    fn usage(&self, message: String, argument: Option<&OsStr>) -> Reading<'a> {
        Reading::Refuse(Refusal::usage(message, argument, self.form()))
    }

    fn help(&self) -> Reading<'a> {
        let words = self.command_words();
        let Some(first) = words.first() else {
            return Reading::Help(help::binary(), self.form());
        };
        let Some(command) = command(first) else {
            return self.unknown(first);
        };
        if let Some(extra) = words.get(1 + command.positionals) {
            return self.extra(command, extra);
        }
        Reading::Help(help::command(command), self.form())
    }

    fn unknown(&self, word: &OsStr) -> Reading<'a> {
        self.usage(
            format!(
                "unknown command `{}`; see plasmid --help",
                word.to_string_lossy()
            ),
            Some(word),
        )
    }

    fn extra(&self, command: &Command, extra: &OsStr) -> Reading<'a> {
        let message = if command.positionals == 0 {
            format!(
                "plasmid {} takes no name, and `{}` is one",
                command.word,
                extra.to_string_lossy()
            )
        } else {
            format!(
                "plasmid {} takes exactly one name, such as `plasmid {} my-plasmid`, and `{}` \
                 is a second one",
                command.word,
                command.word,
                extra.to_string_lossy()
            )
        };
        self.usage(message, Some(extra))
    }

    fn not_served(&self, command: &Command, method: &str) -> Reading<'a> {
        let mut seen: Vec<&str> = Vec::new();
        for flag in self.flags() {
            let Some(form) = [JSON, HUMAN].into_iter().find(|form| flag == *form) else {
                continue;
            };
            if let Some(fault) = self.twice_or_clash(&seen, form) {
                return fault;
            }
            seen.push(form);
        }
        Reading::Refuse(Refusal::not_served(
            method,
            format!("plasmid {} is not served by this build", command.word),
            self.form(),
        ))
    }

    fn twice_or_clash(&self, seen: &[&str], flag: &str) -> Option<Reading<'a>> {
        if seen.contains(&flag) {
            return Some(self.usage(
                format!("{flag} is given twice; a flag is given at most once"),
                Some(OsStr::new(flag)),
            ));
        }
        let other = match flag {
            JSON => HUMAN,
            HUMAN => JSON,
            _ => return None,
        };
        seen.contains(&other).then(|| {
            self.usage(
                format!("{JSON} and {HUMAN} cannot be given together"),
                Some(OsStr::new(flag)),
            )
        })
    }

    fn parse(&self) -> Reading<'a> {
        let command = self.command_words().first().and_then(|word| served(word));
        let mut seen: Vec<&str> = Vec::new();
        let mut at_command = false;
        let mut name: Option<&'a str> = None;
        for &(kind, argument) in &self.head {
            let Some(text) = argument.to_str() else {
                return self.usage(
                    format!(
                        "`{}` is not valid UTF-8, and every argument must be",
                        argument.to_string_lossy()
                    ),
                    Some(argument),
                );
            };
            match (kind, command) {
                (Kind::Value, _) => {}
                (Kind::Flag, _) if !FLAGS.contains(&text) => {
                    return self.usage(
                        format!("unknown flag `{text}`; see plasmid --help"),
                        Some(argument),
                    );
                }
                (Kind::Flag, Some(command)) => return self.not_taken(command, text),
                (Kind::Flag, None) => {
                    if let Some(fault) = self.twice_or_clash(&seen, text) {
                        return fault;
                    }
                    seen.push(text);
                }
                (Kind::Word, None) => return self.unknown(argument),
                (Kind::Word, Some(_)) if !at_command => at_command = true,
                (Kind::Word, Some(_)) if name.is_none() && is_plasmid_name(text) => {
                    name = Some(text);
                }
                (Kind::Word, Some(_)) if name.is_none() => return self.not_a_name(text),
                (Kind::Word, Some(command)) => return self.extra(command, argument),
            }
        }
        if let Some(fault) = self.missing_value() {
            return fault;
        }
        if self.dashes {
            return self.usage(
                "no plasmid command takes arguments after `--`".to_string(),
                Some(OsStr::new("--")),
            );
        }
        match (command, name) {
            (None, _) => self.usage(
                "a command is required; see plasmid --help".to_string(),
                None,
            ),
            (Some(_), None) => self.usage(
                "plasmid new takes exactly one name, such as `plasmid new my-plasmid`".to_string(),
                None,
            ),
            (Some(_), Some(name)) => Reading::New(name),
        }
    }

    fn missing_value(&self) -> Option<Reading<'a>> {
        let (kind, flag) = *self.head.last()?;
        let valued = kind == Kind::Flag && VALUE_FLAGS.iter().any(|known| flag == *known);
        valued.then(|| {
            self.usage(
                format!("{} needs a value", flag.to_string_lossy()),
                Some(flag),
            )
        })
    }

    fn not_taken(&self, command: &Command, flag: &str) -> Reading<'a> {
        self.usage(
            format!(
                "plasmid {} takes no flag but -h and --help, and {flag} is one",
                command.word
            ),
            Some(OsStr::new(flag)),
        )
    }

    fn not_a_name(&self, name: &str) -> Reading<'a> {
        self.usage(
            format!(
                "`{name}` is not a plasmid name, which is lower-case letters and digits in words \
                 joined by single hyphens, at most {NAME_LIMIT} bytes, such as my-plasmid; \
                 nothing was written"
            ),
            Some(OsStr::new(name)),
        )
    }
}

fn holds_its_value(flag: &OsStr) -> bool {
    let bytes = flag.as_encoded_bytes();
    bytes.starts_with(b"--") && bytes.contains(&b'=')
}

fn is_plasmid_name(name: &str) -> bool {
    name.len() <= NAME_LIMIT
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}
