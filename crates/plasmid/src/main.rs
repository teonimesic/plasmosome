mod args;
mod envelope;
mod help;
mod new;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use args::Reading;
use envelope::{Form, Refusal};

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
        Reading::Help(text, form) => print(&text, args::HELP, form, stdout, stderr),
        Reading::Version => print(
            &format!("plasmid {}\n", env!("CARGO_PKG_VERSION")),
            args::VERSION,
            Form::Json,
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

fn print(
    text: &str,
    command: &str,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Exit::Done,
        Err(error) => {
            let failure = Refusal::local_io(
                command,
                format!("could not write to stdout: {error}"),
                error.raw_os_error(),
                form,
            );
            note(stderr, &failure.render());
            failure.exit()
        }
    }
}

fn note(stderr: &mut dyn Write, text: &str) {
    let _ = stderr
        .write_all(text.as_bytes())
        .and_then(|()| stderr.flush());
}
