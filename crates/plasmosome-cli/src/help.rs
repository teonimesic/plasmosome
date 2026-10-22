use crate::args::{Flag, FlagKind};
use crate::commands::Command;

const ABOUT: &str = "\
Usage: plasmosome [FLAGS] COMMAND [FLAGS]

Drives one Plasmosome kernel instance one step at a time. A command with a verb sends that one
verb to the instance's socket and prints one line of JSON: the result on stdout, or the error on
stderr. The registry commands are spec 020's, and they reach no instance socket.
";

const FLAGS: &str = "\
Flags, anywhere among the command words and never after `--`. A flag given twice is a usage
error, and so is --kernel, --socket or --deadline-ms on a registry command. With -h or --help,
only the command words are read: no flag is a fault, and the first bad word is named. A command
this build does not serve reads only --json and --human, and --version reads only itself. Any
other line names its leftmost fault. A command finds its socket by the first of these that is
given: --socket, --kernel, PLASMOSOME_KERNEL. With none, it takes the one instance under
$HOME/.plasmosome/instances/, and refuses when there are several. Beside --socket, --kernel
only names the instance.
  --kernel NAME     the instance named NAME, at $HOME/.plasmosome/instances/NAME/control.uds
  --socket PATH     the socket at PATH, which is absolute with no empty, `.` or `..` component
  --deadline-ms N   the budget for the whole command, from 1 to 3600000 ms; 10000 by default
  --json            one compact JSON line, the default
  --human           output for a person, which nothing may parse
  -h, --help        this help; after command words, that command's or that group's help
  --version         the version, as the sole argument

Environment:
  PLASMOSOME_KERNEL  the instance's name when neither --socket nor --kernel is given; the
                     empty string is a usage error
  HOME               the instances are under $HOME/.plasmosome/instances/

Exit codes:
  0   done: stdout holds the result
  1   unavailable: nothing can have changed, and repeating the command is safe
  2   refused: read the error before trying again
  3   outcome unknown: the request may take effect; observe before acting again
  64  usage: the command line is wrong
  70  internal: the client or the daemon failed; observe first, and report it
";

const SEE_ALSO: &str =
    "Run plasmosome --help for the flags, PLASMOSOME_KERNEL and the exit codes.\n";

pub(crate) fn binary(commands: &[Command]) -> String {
    let mut text = String::from(ABOUT);
    text.push_str("\nCommands, with the verb each sends and whether this build serves it:\n");
    rows(&mut text, commands.iter());
    text.push('\n');
    text.push_str(FLAGS);
    text
}

pub(crate) fn group(commands: &[Command], group: &[&str]) -> String {
    let title = group.join(" ");
    let members = commands
        .iter()
        .filter(|command| command.words.starts_with(group));
    let mut text = format!(
        "Commands under plasmosome {title}, with the verb each sends and whether this build serves it:\n"
    );
    rows(&mut text, members);
    text.push('\n');
    text.push_str(SEE_ALSO);
    text
}

pub(crate) fn command(command: &Command) -> String {
    let changes = if command.changes_state {
        "changes state"
    } else {
        "changes no state"
    };
    let sends = match command.method() {
        Some(method) => format!("Sends the verb {method}, which {changes}."),
        None => format!(
            "Spec 020's registry command {}, which {changes}.",
            command.command
        ),
    };
    let served = if command.served {
        "Served in this build."
    } else {
        "Not served in this build: running it exits 2 with not_served."
    };
    let mut text = format!("Usage: {} [FLAGS]\n\n{sends}\n{served}\n", usage(command));
    if !command.flags.is_empty() {
        let flags: Vec<String> = command.flags.iter().map(written).collect();
        text.push_str(&format!("Its own flags: {}.\n", flags.join(", ")));
    }
    if command.takes_argv {
        text.push_str("Everything after -- is the workload's argv, passed unchanged.\n");
    }
    text.push('\n');
    text.push_str(SEE_ALSO);
    text
}

fn written(flag: &Flag) -> String {
    match flag.kind {
        FlagKind::Text => format!("{} VALUE", flag.name),
        FlagKind::Integer => format!("{} N", flag.name),
        FlagKind::Bare => flag.name.to_string(),
        FlagKind::Mock => format!("{}[=MODE]", flag.name),
    }
}

fn rows<'a>(text: &mut String, commands: impl Iterator<Item = &'a Command> + Clone) {
    let width = commands
        .clone()
        .map(|command| usage(command).len())
        .max()
        .unwrap_or(0);
    for command in commands {
        let verb = command.method().unwrap_or("none");
        let mark = if command.served {
            "served"
        } else {
            "not served"
        };
        text.push_str(&format!("  {:width$}  {verb:19}  {mark}\n", usage(command)));
    }
}

fn usage(command: &Command) -> String {
    let mut usage = String::from("plasmosome");
    for word in command.words.iter().chain(command.operands) {
        usage.push(' ');
        usage.push_str(word);
    }
    usage
}
