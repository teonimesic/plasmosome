use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};

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

const KNOWN_FLAGS: [&str; 5] = [KERNEL, SOCKET, DEADLINE, JSON, HUMAN];

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
/// exit. A panic prints nothing of its own: it ends in one `internal` envelope on `stderr` and
/// [`Exit::Internal`].
pub fn handle(
    args: &[OsString],
    version: &str,
    commands: &[Command],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    quiet_panics();
    let mut named = INVALID.to_string();
    QUIET.set(true);
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        let reading = read(args, commands);
        named = reading.command().to_string();
        act(reading, version, stdout, stderr)
    }));
    QUIET.set(false);
    outcome.unwrap_or_else(|_| {
        let error = client(
            ClientCode::Internal,
            "the client failed; please report it".to_string(),
            Vec::new(),
        );
        refuse(named, error, Form::Json, stdout, stderr)
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
    if let Named::Command(command) = name(&line.words(), commands)
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
    args: &'a [OsString],
    head: Vec<Arg<'a>>,
    tail: &'a [OsString],
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
            args,
            head: classified,
            tail,
        }
    }

    fn words(&self) -> Vec<&'a OsStr> {
        self.head
            .iter()
            .filter(|arg| arg.kind == Kind::Word)
            .map(|arg| arg.text)
            .collect()
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
        let words = self.words();
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
        if let Some(bad) = self.args.iter().find(|arg| arg.to_str().is_none()) {
            return self.usage(
                format!(
                    "`{}` is not valid UTF-8, and every argument must be",
                    bad.to_string_lossy()
                ),
                Some(bad),
            );
        }
        if let Some(fault) = self.flag_fault() {
            return fault;
        }
        let words = self.words();
        let command = match name(&words, commands) {
            Named::Command(command) => command,
            Named::Nothing => {
                return self.usage(
                    "a command is required; see plasmosome --help".to_string(),
                    None,
                );
            }
            Named::Group(group) => {
                let group = group.join(" ");
                return self.usage(
                    format!("`{group}` names a group of commands; see plasmosome {group} --help"),
                    words.last().copied(),
                );
            }
            Named::Breaks(index) => return self.unknown_command(words[index]),
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
        if let Some(extra) = self.extra_word(&command, &words) {
            return extra;
        }
        self.values(command)
    }

    fn flag_fault(&self) -> Option<Reading> {
        let mut seen: Vec<&OsStr> = Vec::new();
        for (index, flag) in self.flags() {
            if !KNOWN_FLAGS.iter().any(|known| flag == *known) {
                return Some(self.usage(
                    format!(
                        "unknown flag `{}`; see plasmosome --help",
                        flag.to_string_lossy()
                    ),
                    Some(flag),
                ));
            }
            if let Some(fault) = self.repeated(&seen, flag) {
                return Some(fault);
            }
            let valued = VALUE_FLAGS.iter().any(|known| flag == *known);
            if valued && self.head.get(index + 1).is_none() {
                return Some(self.usage(
                    format!("{} needs a value", flag.to_string_lossy()),
                    Some(flag),
                ));
            }
            seen.push(flag);
        }
        None
    }

    fn values(&self, command: Command) -> Reading {
        let mut invocation = Invocation {
            command,
            kernel: None,
            socket: None,
            deadline_ms: None,
            form: self.form(),
        };
        if let Some(name) = self.value(KERNEL) {
            if !is_instance_name(&name) {
                return self.refused(
                    KERNEL,
                    &name,
                    "an instance name is not empty, `.` or `..`, and holds no `/`, backslash, \
                     NUL or terminal control",
                );
            }
            invocation.kernel = Some(name);
        }
        if let Some(path) = self.value(SOCKET) {
            if !is_normal_absolute(&path) {
                return self.refused(
                    SOCKET,
                    &path,
                    "a socket path is absolute, with no empty, `.` or `..` component",
                );
            }
            invocation.socket = Some(path);
        }
        if let Some(text) = self.value(DEADLINE) {
            let Some(deadline_ms) = deadline_ms(&text) else {
                return self.refused(
                    DEADLINE,
                    &text,
                    &format!("a deadline is an integer from 1 to {MAX_DEADLINE_MS}"),
                );
            };
            invocation.deadline_ms = Some(deadline_ms);
        }
        Reading::Run(invocation)
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
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if !QUIET.get() {
            previous(info);
        }
    }));
}

fn act(reading: Reading, version: &str, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Exit {
    #[cfg(test)]
    if tests::PANIC_IN_COMMAND.get() {
        panic!("the panic a test asked for");
    }
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::panic;

    use super::handle;
    use crate::codes::Exit;
    use crate::commands::COMMANDS;

    thread_local! {
        pub(super) static PANIC_IN_COMMAND: Cell<bool> = const { Cell::new(false) };
        static HOOK_RAN: Cell<bool> = const { Cell::new(false) };
    }

    #[test]
    fn a_panic_is_internal() {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            HOOK_RAN.set(true);
            previous(info);
        }));
        let args: Vec<OsString> = vec!["cell".into(), "list".into()];
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        PANIC_IN_COMMAND.set(true);
        let exit = handle(&args, "0.0.0", COMMANDS, &mut stdout, &mut stderr);
        PANIC_IN_COMMAND.set(false);
        assert_eq!(exit, Exit::Internal);
        assert!(
            !HOOK_RAN.get(),
            "the panic reached the hook that prints its message"
        );
        assert!(stdout.is_empty());
        let text = String::from_utf8(stderr).expect("stderr is UTF-8");
        assert!(
            text.ends_with('\n') && text.matches('\n').count() == 1,
            "{text:?}"
        );
        assert!(
            !text.contains("asked for"),
            "the panic's text reached stderr: {text:?}"
        );
        let envelope: serde_json::Value =
            serde_json::from_str(&text).expect("stderr is one JSON envelope");
        assert_eq!(envelope["command"], "cell.list");
        assert_eq!(envelope["error"]["domain"], "client");
        assert_eq!(envelope["error"]["code"], "internal");
        assert!(envelope.get("instance").is_none());
    }
}
