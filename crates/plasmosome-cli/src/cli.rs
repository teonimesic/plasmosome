use std::ffi::OsString;
use std::io::{self, Write};

use crate::codes::Exit;
use crate::commands::{COMMANDS, Command};
use crate::envelope::{ClientError, Form};

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
    handle(
        &args,
        version,
        COMMANDS,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
    .code()
}

/// Reads `args` against `commands`, writes the outcome to `stdout` or `stderr`, and returns the
/// exit.
pub fn handle(
    _args: &[OsString],
    _version: &str,
    _commands: &[Command],
    _stdout: &mut dyn Write,
    _stderr: &mut dyn Write,
) -> Exit {
    Exit::Done
}

/// Reads a command line in spec 025 §2's order.
pub fn read(_args: &[OsString], _commands: &[Command]) -> Reading {
    Reading::Version
}
