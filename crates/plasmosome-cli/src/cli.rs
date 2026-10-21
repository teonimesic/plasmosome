use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use plasmosome_core::state::InstanceName;

use crate::codes::{ClientCode, Exit};
use crate::commands::{COMMANDS, Command};
use crate::envelope::{ClientError, Envelope, ErrorBody, Form, emit};
use crate::help;
use crate::json::{self, Json, Number};

const INVALID: &str = "invalid";

const SHORT_HELP: &str = "-h";

const HELP: &str = "--help";

const VERSION: &str = "--version";

const JSON: &str = "--json";

const HUMAN: &str = "--human";

const KERNEL: &str = "--kernel";

const SOCKET: &str = "--socket";

const DEADLINE: &str = "--deadline-ms";

const VALUE_FLAGS: [&str; 3] = [KERNEL, SOCKET, DEADLINE];

const KNOWN_FLAGS: [&str; 8] = [
    KERNEL, SOCKET, DEADLINE, JSON, HUMAN, SHORT_HELP, HELP, VERSION,
];

const MAX_DEADLINE_MS: u32 = 3_600_000;

const MAX_DEADLINE_DIGITS: usize = 7;

thread_local! {
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// What a command line asks for, read in spec 025 §2's order: help, then the version, then a
/// command this build does not serve, then the full parse.
#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    /// Print `text` on stdout and exit 0. A failure to print it is written in `form`.
    Help { text: String, form: Form },
    /// Print `plasmosome VERSION` on stdout and exit 0.
    Version,
    /// Write `error` on stderr in `form`, under `command`, and exit with its code's exit.
    Refuse {
        command: String,
        error: ClientError,
        form: Form,
    },
    /// Run a served command.
    Run(Invocation),
}

/// A served command's line, after the full parse found no fault in it.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub command: Command,
    /// `--kernel`'s name, checked against the instance name rules.
    pub kernel: Option<String>,
    /// `--socket`'s path, absolute and normal.
    pub socket: Option<String>,
    /// `--deadline-ms`, from 1 to 3,600,000. `None` leaves the command its default.
    pub deadline_ms: Option<u32>,
    pub form: Form,
}

/// The whole `main` of the `plasmosome` binary. Takes the arguments after the program name and
/// the package version the binary was built with, and returns the process exit code.
pub fn main_with(args: impl IntoIterator<Item = OsString>, version: &str) -> u8 {
    let args: Vec<OsString> = args.into_iter().collect();
    let stdout = io::stdout();
    let stderr = io::stderr();
    handle(
        &args,
        version,
        COMMANDS,
        &mut stdout.lock(),
        &mut stderr.lock(),
    )
    .code()
}

/// Reads `args` against `commands`, writes the outcome to `stdout` or `stderr`, and returns the
/// exit. A panic prints nothing of its own: it ends in one `internal` envelope on `stderr`, in
/// the form the line chose, and [`Exit::Internal`].
///
/// The first call installs a panic hook for the whole process. The hook is silent on a thread
/// while it runs `handle`, and passes every other panic to the hook that was set before it. A
/// hook set after that first call replaces it, and a panic in a command then reaches that hook.
pub fn handle(
    args: &[OsString],
    version: &str,
    commands: &[Command],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    quiet_panics();
    let mut named = INVALID.to_string();
    let mut form = Form::Json;
    let quiet = QUIET.replace(true);
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        let reading = read(args, commands);
        named = reading.command().to_string();
        form = reading.form();
        act(reading, version, stdout, stderr)
    }));
    QUIET.set(quiet);
    outcome.unwrap_or_else(|_| {
        let error = client(
            ClientCode::Internal,
            "the client failed; please report it".to_string(),
            Vec::new(),
        );
        refuse(named, error, form, stdout, stderr)
    })
}

/// Reads a command line in spec 025 §2's order. `args` are the arguments after the program name;
/// `commands` is the table to read them against, [`COMMANDS`] in the binary.
pub fn read(args: &[OsString], commands: &[Command]) -> Reading {
    let line = Line::new(args);
    if line
        .head
        .iter()
        .any(|arg| arg.text == SHORT_HELP || arg.text == HELP)
    {
        return line.help(commands);
    }
    if line.head.iter().any(|arg| arg.text == VERSION) {
        if args.len() == 1 {
            return Reading::Version;
        }
        return line.usage(
            format!("{VERSION} is read only as the sole argument"),
            Some(OsStr::new(VERSION)),
        );
    }
    if let Named::Command(command) = name(&line.command_words().texts, commands)
        && !command.served
    {
        return line.not_served(command);
    }
    line.parse(commands)
}

impl Reading {
    fn command(&self) -> &str {
        match self {
            Reading::Help { .. } => HELP,
            Reading::Version => VERSION,
            Reading::Refuse { command, .. } => command,
            Reading::Run(invocation) => invocation.command.command,
        }
    }

    fn form(&self) -> Form {
        match self {
            Reading::Help { form, .. } | Reading::Refuse { form, .. } => *form,
            Reading::Version => Form::Json,
            Reading::Run(invocation) => invocation.form,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Flag,
    Value,
}

struct Arg<'a> {
    text: &'a OsStr,
    kind: Kind,
}

struct Line<'a> {
    head: Vec<Arg<'a>>,
    tail: &'a [OsString],
}

struct Words<'a> {
    texts: Vec<&'a OsStr>,
    positions: Vec<usize>,
}

enum Named {
    Nothing,
    Group(&'static [&'static str]),
    Command(Command),
    Breaks(usize),
}

impl<'a> Line<'a> {
    fn new(args: &'a [OsString]) -> Line<'a> {
        let split = args
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(args.len());
        let (head, tail) = args.split_at(split);
        let mut classified = Vec::with_capacity(head.len());
        let mut wants_value = false;
        for text in head {
            let kind = if wants_value {
                Kind::Value
            } else if text.as_encoded_bytes().starts_with(b"-") {
                Kind::Flag
            } else {
                Kind::Word
            };
            wants_value = kind == Kind::Flag && VALUE_FLAGS.iter().any(|flag| text == *flag);
            classified.push(Arg {
                text: text.as_os_str(),
                kind,
            });
        }
        Line {
            head: classified,
            tail,
        }
    }

    fn command_words(&self) -> Words<'a> {
        let (positions, texts) = self
            .head
            .iter()
            .enumerate()
            .take_while(|(_, arg)| {
                arg.kind != Kind::Flag || is_known(arg.text) || holds_its_value(arg.text)
            })
            .filter(|(_, arg)| arg.kind == Kind::Word)
            .map(|(index, arg)| (index, arg.text))
            .unzip();
        Words { texts, positions }
    }

    fn flags(&self) -> impl Iterator<Item = (usize, &'a OsStr)> + '_ {
        self.head
            .iter()
            .enumerate()
            .filter(|(_, arg)| arg.kind == Kind::Flag)
            .map(|(index, arg)| (index, arg.text))
    }

    fn form(&self) -> Form {
        let count = |flag: &str| self.flags().filter(|(_, text)| *text == flag).count();
        if count(HUMAN) == 1 && count(JSON) == 0 {
            Form::Human
        } else {
            Form::Json
        }
    }

    fn usage(&self, message: String, argument: Option<&OsStr>) -> Reading {
        let fields = argument
            .map(|argument| {
                vec![(
                    "argument".to_string(),
                    Json::String(argument.to_string_lossy().into_owned()),
                )]
            })
            .unwrap_or_default();
        Reading::Refuse {
            command: INVALID.to_string(),
            error: client(ClientCode::InvalidArgument, message, fields),
            form: self.form(),
        }
    }

    fn help(&self, commands: &[Command]) -> Reading {
        let words = self.command_words().texts;
        let text = match name(&words, commands) {
            Named::Nothing => help::binary(commands),
            Named::Group(group) => help::group(commands, group),
            Named::Command(command) => {
                if let Some(extra) = self.extra_word(&command, &words) {
                    return extra;
                }
                help::command(&command)
            }
            Named::Breaks(index) => return self.unknown_command(words[index]),
        };
        Reading::Help {
            text,
            form: self.form(),
        }
    }

    fn extra_word(&self, command: &Command, words: &[&OsStr]) -> Option<Reading> {
        let extra = words.get(command.words.len() + command.operands.len())?;
        Some(self.usage(
            format!(
                "plasmosome {} takes no argument `{}`",
                command.words.join(" "),
                extra.to_string_lossy()
            ),
            Some(extra),
        ))
    }

    fn unknown_command(&self, word: &OsStr) -> Reading {
        self.usage(
            format!(
                "unknown command `{}`; see plasmosome --help",
                word.to_string_lossy()
            ),
            Some(word),
        )
    }

    fn not_served(&self, command: Command) -> Reading {
        if let Some(fault) = self.form_fault() {
            return fault;
        }
        let fields = command
            .method()
            .map(|method| vec![("method".to_string(), Json::String(method.to_string()))])
            .unwrap_or_default();
        Reading::Refuse {
            command: command.command.to_string(),
            error: client(
                ClientCode::NotServed,
                format!(
                    "plasmosome {} is not served by this build",
                    command.words.join(" ")
                ),
                fields,
            ),
            form: self.form(),
        }
    }

    fn form_fault(&self) -> Option<Reading> {
        let mut seen: Vec<&OsStr> = Vec::new();
        for (_, flag) in self.flags() {
            if flag != JSON && flag != HUMAN {
                continue;
            }
            if let Some(fault) = self.repeated(&seen, flag) {
                return Some(fault);
            }
            seen.push(flag);
        }
        None
    }

    fn repeated(&self, seen: &[&OsStr], flag: &OsStr) -> Option<Reading> {
        if seen.contains(&flag) {
            return Some(self.usage(
                format!("{} is given twice", flag.to_string_lossy()),
                Some(flag),
            ));
        }
        let clashes = (flag == JSON && seen.contains(&OsStr::new(HUMAN)))
            || (flag == HUMAN && seen.contains(&OsStr::new(JSON)));
        clashes.then(|| {
            self.usage(
                format!("{JSON} and {HUMAN} cannot be given together"),
                Some(flag),
            )
        })
    }

    fn parse(&self, commands: &[Command]) -> Reading {
        let words = self.command_words();
        let named = name(&words.texts, commands);
        let mut seen: Vec<&OsStr> = Vec::new();
        for (index, arg) in self.head.iter().enumerate() {
            let fault = if arg.text.to_str().is_none() {
                Some(self.not_utf8(arg.text))
            } else {
                match arg.kind {
                    Kind::Flag => self.flag_fault(index, &seen, &named),
                    Kind::Value => self.value_fault(self.head[index - 1].text, arg.text),
                    Kind::Word => self.word_fault(index, &words, &named),
                }
            };
            if let Some(fault) = fault {
                return fault;
            }
            if arg.kind == Kind::Flag {
                seen.push(arg.text);
            }
        }
        let Named::Command(command) = named else {
            return self.no_command();
        };
        if let Some(dashes) = self.tail.first() {
            return self.usage(
                format!(
                    "plasmosome {} takes nothing after `--`",
                    command.words.join(" ")
                ),
                Some(dashes),
            );
        }
        self.invocation(command)
    }

    fn not_utf8(&self, bad: &OsStr) -> Reading {
        self.usage(
            format!(
                "`{}` is not valid UTF-8, and every argument must be",
                bad.to_string_lossy()
            ),
            Some(bad),
        )
    }

    fn no_command(&self) -> Reading {
        if let Some(bad) = self.tail.iter().find(|arg| arg.to_str().is_none()) {
            return self.not_utf8(bad);
        }
        self.usage(
            "a command is required; see plasmosome --help".to_string(),
            None,
        )
    }

    fn flag_fault(&self, index: usize, seen: &[&OsStr], named: &Named) -> Option<Reading> {
        let flag = self.head[index].text;
        if !is_known(flag) {
            return Some(self.usage(
                format!(
                    "unknown flag `{}`; see plasmosome --help",
                    flag.to_string_lossy()
                ),
                Some(flag),
            ));
        }
        if let Some(fault) = self.repeated(seen, flag) {
            return Some(fault);
        }
        if !VALUE_FLAGS.iter().any(|known| flag == *known) {
            return None;
        }
        if let Named::Command(command) = named
            && command.method().is_none()
        {
            return Some(self.usage(
                format!(
                    "plasmosome {} reaches no control socket, so it takes no {}",
                    command.words.join(" "),
                    flag.to_string_lossy()
                ),
                Some(flag),
            ));
        }
        self.head.get(index + 1).is_none().then(|| {
            self.usage(
                format!("{} needs a value", flag.to_string_lossy()),
                Some(flag),
            )
        })
    }

    fn value_fault(&self, flag: &OsStr, value: &OsStr) -> Option<Reading> {
        let text = value.to_string_lossy();
        if flag == KERNEL && !is_instance_name(&text) {
            return Some(self.refused(
                KERNEL,
                &text,
                "an instance name is not empty, `.` or `..`, and holds no `/`, backslash, \
                 NUL or terminal control",
            ));
        }
        if flag == SOCKET && !is_normal_absolute(&text) {
            return Some(self.refused(
                SOCKET,
                &text,
                "a socket path is absolute, with no empty, `.` or `..` component",
            ));
        }
        if flag == DEADLINE && deadline_ms(&text).is_none() {
            return Some(self.refused(
                DEADLINE,
                &text,
                &format!("a deadline is an integer from 1 to {MAX_DEADLINE_MS}"),
            ));
        }
        None
    }

    fn word_fault(&self, index: usize, words: &Words<'a>, named: &Named) -> Option<Reading> {
        let at = words
            .positions
            .iter()
            .position(|position| *position == index)?;
        let word = words.texts[at];
        match named {
            Named::Breaks(breaks) if *breaks == at => Some(self.unknown_command(word)),
            Named::Group(group) if at + 1 == words.texts.len() => {
                let group = group.join(" ");
                Some(self.usage(
                    format!("`{group}` names a group of commands; see plasmosome {group} --help"),
                    Some(word),
                ))
            }
            Named::Command(command) if at == command.words.len() + command.operands.len() => {
                self.extra_word(command, &words.texts)
            }
            _ => None,
        }
    }

    fn invocation(&self, command: Command) -> Reading {
        Reading::Run(Invocation {
            command,
            kernel: self.value(KERNEL),
            socket: self.value(SOCKET),
            deadline_ms: self.value(DEADLINE).as_deref().and_then(deadline_ms),
            form: self.form(),
        })
    }

    fn value(&self, flag: &str) -> Option<String> {
        let (index, _) = self.flags().find(|(_, text)| *text == flag)?;
        let value = self.head.get(index + 1)?;
        Some(value.text.to_string_lossy().into_owned())
    }

    fn refused(&self, flag: &str, value: &str, rule: &str) -> Reading {
        self.usage(
            format!("`{value}` is refused for {flag}: {rule}"),
            Some(OsStr::new(value)),
        )
    }
}

fn name(words: &[&OsStr], commands: &[Command]) -> Named {
    let mut group = None;
    for taken in 1..=words.len() {
        let prefix = &words[..taken];
        let leads = |command: &&Command| {
            command.words.len() >= taken
                && command
                    .words
                    .iter()
                    .zip(prefix)
                    .all(|(word, given)| *given == *word)
        };
        let mut leading = commands.iter().filter(leads).peekable();
        let Some(first) = leading.peek() else {
            return Named::Breaks(taken - 1);
        };
        group = Some(&first.words[..taken]);
        if let Some(command) = leading.find(|command| command.words.len() == taken) {
            return Named::Command(*command);
        }
    }
    group.map_or(Named::Nothing, Named::Group)
}

fn is_known(flag: &OsStr) -> bool {
    KNOWN_FLAGS.iter().any(|known| flag == *known)
}

fn holds_its_value(flag: &OsStr) -> bool {
    let bytes = flag.as_encoded_bytes();
    bytes.starts_with(b"--") && bytes.contains(&b'=')
}

fn is_instance_name(text: &str) -> bool {
    InstanceName::parse(text).is_ok() && !text.chars().any(json::is_terminal_control)
}

fn is_normal_absolute(path: &str) -> bool {
    path.strip_prefix('/').is_some_and(|rest| {
        rest.split('/')
            .all(|component| !matches!(component, "" | "." | ".."))
    })
}

fn deadline_ms(text: &str) -> Option<u32> {
    if text.is_empty()
        || text.len() > MAX_DEADLINE_DIGITS
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    text.parse()
        .ok()
        .filter(|deadline_ms| (1..=MAX_DEADLINE_MS).contains(deadline_ms))
}

fn quiet_panics() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !QUIET.get() {
                previous(info);
            }
        }));
    });
}

fn act(reading: Reading, version: &str, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Exit {
    match reading {
        Reading::Help { text, form } => print(&text, HELP, form, stdout, stderr),
        Reading::Version => print(
            &format!("plasmosome {version}\n"),
            VERSION,
            Form::Json,
            stdout,
            stderr,
        ),
        Reading::Refuse {
            command,
            error,
            form,
        } => refuse(command, error, form, stdout, stderr),
        Reading::Run(invocation) => {
            let error = client(
                ClientCode::Internal,
                format!(
                    "this build marks {} served and has no code for it",
                    invocation.command.command
                ),
                Vec::new(),
            );
            refuse(
                invocation.command.command.to_string(),
                error,
                invocation.form,
                stdout,
                stderr,
            )
        }
    }
}

fn print(
    text: &str,
    command: &str,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let Err(error) = stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    else {
        return Exit::Done;
    };
    let fields = error
        .raw_os_error()
        .map(|errno| {
            vec![(
                "errno".to_string(),
                Json::Number(Number::new(errno.to_string()).expect("an integer is a JSON number")),
            )]
        })
        .unwrap_or_default();
    let error = client(
        ClientCode::LocalIo,
        format!("could not write to stdout: {error}"),
        fields,
    );
    refuse(command.to_string(), error, form, stdout, stderr)
}

fn refuse(
    command: String,
    error: ClientError,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let exit = error.code().exit();
    let envelope = Envelope {
        command,
        instance: None,
        outcome: Err(ErrorBody::Client(error)),
    };
    let _ = emit(&envelope, form, stdout, stderr);
    exit
}

fn client(code: ClientCode, message: String, fields: Vec<(String, Json)>) -> ClientError {
    ClientError::new(code, message, fields)
        .expect("the client's own fields repeat no key and hold no object")
}
