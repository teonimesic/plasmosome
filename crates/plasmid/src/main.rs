mod args;
mod envelope;
mod help;
mod new;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use args::Reading;

#[derive(Clone, Copy)]
enum Exit {
    Done = 0,
    Unavailable = 1,
    Refused = 2,
    OutcomeUnknown = 3,
    Usage = 64,
}

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let exit = run(
        &arguments,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    ExitCode::from(exit as u8)
}

fn run(arguments: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> Exit {
    match args::read(arguments) {
        Reading::Help(text) => print(&text, stdout, stderr),
        Reading::Version => print(
            &format!("plasmid {}\n", env!("CARGO_PKG_VERSION")),
            stdout,
            stderr,
        ),
        Reading::Refuse(refusal) => {
            note(stderr, &refusal.render());
            refusal.exit()
        }
        Reading::New(name) => new::run(name, stdout, stderr),
    }
}

fn print(text: &str, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Exit {
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Exit::Done,
        Err(error) => {
            note(
                stderr,
                &format!("plasmid: cannot write to stdout: {error}; nothing changed\n"),
            );
            Exit::Unavailable
        }
    }
}

fn note(stderr: &mut dyn Write, text: &str) {
    let _ = stderr
        .write_all(text.as_bytes())
        .and_then(|()| stderr.flush());
}
