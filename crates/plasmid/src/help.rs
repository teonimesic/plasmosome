use crate::args::{COMMANDS, Command, Serving};

const OVERVIEW: &str = "\
plasmid — the command line for plasmid authors

Usage: plasmid [flags] <command> [<name>]

Commands:
";

const NEW_SUMMARY: &str = "write <name>.toml, the plasmid's declaration";

const DETAILS: &str = "
plasmid new <name> writes <name>.toml, the plasmid's declaration, in the current directory, and
prints the sections it still needs; see plasmid new --help. A command that is not served exits 2
with not_served, reads no environment variable and reaches no socket. plasmid <command> --help
describes one command.

Global flags, anywhere among the command words, never after --:
  --kernel NAME      the kernel instance a command reaches
  --socket PATH      the control socket a command reaches, absolute with no empty, . or .. component
  --deadline-ms N    the time a command may take, from 1 to 3600000 milliseconds
  --json             the JSON form, the default
  --human            a form for a person, which nothing may parse
  -h, --help         this help, or a command's help with its words
  --version          the version, as the sole argument

--kernel, --socket and --deadline-ms are for commands that reach a socket, and none of those is
served yet. Each takes the next argument as its value, even one that starts with -, but never --.
A command that is not served reads only --json and --human of these flags, and plasmid new takes
none of them but -h and --help.

With -h or --help before --, only the command words are judged. No flag is a fault then, known or
unknown, with or without a value, given once or twice, and a usage error names the first bad word.
Without them, --version anywhere before -- but as the sole argument is a usage error. Otherwise a
usage error names the first fault on the line, flag, value or word, reading from the left, and a
flag given twice is one.

Environment:
  PLASMOSOME_KERNEL  works as --kernel, for commands that reach a socket

Exit codes:
  0   done; stdout holds the result
  1   unavailable; nothing can have changed, so the same command can be run again
  2   refused, by the kernel, the registry or plasmid's own checks, or not served
  3   outcome unknown; look at what the command acts on before acting again
  64  usage; the command line is wrong
  70  internal; plasmid or the daemon broke the protocol or failed
";

const NEW: &str = "\
plasmid new <name>

Writes <name>.toml, the plasmid's declaration, in the current directory. It fills in the id, the
version and an empty description, and grants nothing. It prints the sections the declaration
still needs on stdout, one per line, and writes no component until the plasmid interface is
frozen. A name is lower-case letters and digits in words joined by single hyphens, at most 64
bytes. It never overwrites. It takes none of the global flags but -h and --help.

Exit codes:
  2   wrote <name>.toml and printed the sections it still needs; no component is written
  1   <name>.toml already exists, as a file, a directory or a symlink, or cannot be created;
      nothing was written
  3   <name>.toml was created but could not be finished, or the sections could not be printed;
      look at <name>.toml before running it again
  64  the command line is wrong: no name, a second name, a name that is not a plasmid name, or a
      flag other than -h and --help; nothing was written
";

pub(crate) fn binary() -> String {
    let mut text = OVERVIEW.to_string();
    for command in &COMMANDS {
        let (mark, summary) = match command.serving {
            Serving::Local => ("served", NEW_SUMMARY),
            Serving::NotServed { method } => ("not served", method),
        };
        text.push_str(&format!(
            "  {:<15}  {mark:<10}  {summary}\n",
            command.usage()
        ));
    }
    text.push_str(DETAILS);
    text
}

pub(crate) fn command(command: &Command) -> String {
    match command.serving {
        Serving::Local => NEW.to_string(),
        Serving::NotServed { method } => format!(
            "plasmid {}\n\n\
             Its wire method is {method}. It is not served by this build: it exits 2\n\
             with not_served, reads no environment variable and reaches no socket.\n\
             See plasmid --help for the global flags and the exit codes.\n",
            command.usage()
        ),
    }
}
