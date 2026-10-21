use std::ffi::{OsStr, OsString};

use crate::envelope::{Form, Refusal};
use crate::help;

const SHORT_HELP: &str = "-h";

const HELP: &str = "--help";

const VERSION: &str = "--version";

const JSON: &str = "--json";

const HUMAN: &str = "--human";

const VALUE_FLAGS: [&str; 3] = ["--kernel", "--socket", "--deadline-ms"];

const FLAGS: [&str; 5] = ["--kernel", "--socket", "--deadline-ms", JSON, HUMAN];

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
    Help(String),
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
    arguments: &'a [OsString],
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
    if let Some((command, method)) = line.words().first().and_then(|word| unserved(word)) {
        return line.not_served(command, method);
    }
    line.parse()
}

fn command(word: &OsStr) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| word == command.word)
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
            arguments,
            head,
            dashes: split < arguments.len(),
        }
    }

    fn anywhere(&self, flag: &str) -> bool {
        self.head.iter().any(|(_, argument)| *argument == flag)
    }

    fn of(&self, wanted: Kind) -> impl Iterator<Item = (usize, &'a OsStr)> + '_ {
        self.head
            .iter()
            .enumerate()
            .filter(move |(_, (kind, _))| *kind == wanted)
            .map(|(index, (_, argument))| (index, *argument))
    }

    fn words(&self) -> Vec<&'a OsStr> {
        self.of(Kind::Word).map(|(_, word)| word).collect()
    }

    fn count(&self, flag: &str) -> usize {
        self.of(Kind::Flag)
            .filter(|(_, argument)| *argument == flag)
            .count()
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
        let words = self.words();
        let Some(first) = words.first() else {
            return Reading::Help(help::binary());
        };
        let Some(command) = command(first) else {
            return self.unknown(first);
        };
        if let Some(extra) = words.get(1 + command.positionals) {
            return self.extra(command, extra);
        }
        Reading::Help(help::command(command))
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
        if let Some(fault) = self.repeated(&[JSON, HUMAN]).or_else(|| self.clash()) {
            return fault;
        }
        Reading::Refuse(Refusal::not_served(
            method,
            format!("plasmid {} is not served by this build", command.word),
            self.form(),
        ))
    }

    fn repeated(&self, among: &[&str]) -> Option<Reading<'a>> {
        let mut seen: Vec<&OsStr> = Vec::new();
        for (_, flag) in self.of(Kind::Flag) {
            if !among.iter().any(|known| flag == *known) {
                continue;
            }
            if seen.contains(&flag) {
                return Some(self.usage(
                    format!(
                        "{} is given twice; a flag is given at most once",
                        flag.to_string_lossy()
                    ),
                    Some(flag),
                ));
            }
            seen.push(flag);
        }
        None
    }

    fn clash(&self) -> Option<Reading<'a>> {
        let at = |wanted: &str| {
            self.of(Kind::Flag)
                .find(|(_, flag)| *flag == wanted)
                .map(|(index, _)| index)
        };
        let (json, human) = (at(JSON)?, at(HUMAN)?);
        let later = if json > human { JSON } else { HUMAN };
        Some(self.usage(
            format!("{JSON} and {HUMAN} cannot be given together"),
            Some(OsStr::new(later)),
        ))
    }

    fn parse(&self) -> Reading<'a> {
        if let Some(bad) = self
            .arguments
            .iter()
            .find(|argument| argument.to_str().is_none())
        {
            return self.usage(
                format!(
                    "`{}` is not valid UTF-8, and every argument must be",
                    bad.to_string_lossy()
                ),
                Some(bad),
            );
        }
        if let Some((_, flag)) = self
            .of(Kind::Flag)
            .find(|(_, flag)| !FLAGS.iter().any(|known| flag == known))
        {
            return self.usage(
                format!(
                    "unknown flag `{}`; see plasmid --help",
                    flag.to_string_lossy()
                ),
                Some(flag),
            );
        }
        if self.dashes {
            return self.usage(
                "no plasmid command takes arguments after `--`".to_string(),
                Some(OsStr::new("--")),
            );
        }
        if let Some(fault) = self
            .repeated(&FLAGS)
            .or_else(|| self.missing_value())
            .or_else(|| self.clash())
        {
            return fault;
        }
        let words = self.words();
        let Some(first) = words.first() else {
            return self.usage(
                "a command is required; see plasmid --help".to_string(),
                None,
            );
        };
        let Some(command) = command(first) else {
            return self.unknown(first);
        };
        self.parse_new(command, &words)
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

    fn parse_new(&self, command: &Command, words: &[&'a OsStr]) -> Reading<'a> {
        if let Some((_, flag)) = self.of(Kind::Flag).next() {
            return self.usage(
                format!(
                    "plasmid new takes none of the global flags, and {} is one",
                    flag.to_string_lossy()
                ),
                Some(flag),
            );
        }
        let Some(name) = words.get(1) else {
            return self.usage(
                "plasmid new takes exactly one name, such as `plasmid new my-plasmid`".to_string(),
                None,
            );
        };
        if let Some(extra) = words.get(2) {
            return self.extra(command, extra);
        }
        match name.to_str().filter(|name| is_plasmid_name(name)) {
            Some(name) => Reading::New(name),
            None => self.usage(
                format!(
                    "`{}` is not a plasmid name, which is lower-case letters and digits in words \
                     joined by single hyphens, at most {NAME_LIMIT} bytes, such as my-plasmid; \
                     nothing was written",
                    name.to_string_lossy()
                ),
                Some(name),
            ),
        }
    }
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
