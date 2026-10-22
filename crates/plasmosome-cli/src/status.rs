use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use crate::cli::{Context, INVALID, Invocation, client, local_io, refuse};
use crate::clock::Deadline;
use crate::codes::{ClientCode, Exit};
use crate::commands::Command;
use crate::envelope::{EmitError, Envelope, ErrorBody, Form, Stream, emit};
use crate::exchange::{Unreplied, exchange};
use crate::json::Json;
use crate::reply::{Reply, cut_off, judge_reply, kernel_exit, request_line, unanswered};
use crate::select::{self, Named, Selected, Target, Unselected, VARIABLE};

const DEFAULT_DEADLINE_MS: u32 = 10_000;

const MAX_REQUEST_BYTES: usize = 1_048_576;

pub(crate) fn run(
    invocation: Invocation,
    context: &Context<'_>,
    start: Instant,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let Invocation {
        command,
        kernel,
        socket,
        deadline_ms,
        form,
        ..
    } = invocation;
    let target = match select::target(socket, kernel, context.env) {
        Ok(target) => target,
        Err(error) => {
            let named = if error.code() == ClientCode::InvalidArgument {
                INVALID
            } else {
                command.command
            };
            return refuse(named.to_string(), error, form, stdout, stderr);
        }
    };
    let deadline = Deadline::new(
        context.clock,
        start,
        deadline_ms.unwrap_or(DEFAULT_DEADLINE_MS),
    );
    let uid = context.trusted_uid;
    let (instance, outcome) = match target {
        Target::Named(named) => {
            let request = match request(&command, &named.instance.name) {
                Ok(request) => request,
                Err(bytes) => return too_large(bytes, named, form, stdout, stderr),
            };
            let socket = Path::new(&named.instance.socket);
            let outcome = select::connect(socket, uid, named.origin)
                .map_err(ErrorBody::Client)
                .and_then(|stream| answer(&command, stream, socket, &request, uid, &deadline));
            (Some(named.instance), outcome)
        }
        Target::Default(instances) => match select::select_default(&instances, uid) {
            Ok(Selected { stream, instance }) => {
                let request = request(&command, &instance.name)
                    .expect("a name read from a directory keeps the request line under the bound");
                let socket = Path::new(&instance.socket);
                let outcome = answer(&command, stream, socket, &request, uid, &deadline);
                (Some(instance), outcome)
            }
            Err(Unselected { error, instance }) => (instance, Err(ErrorBody::Client(error))),
        },
    };
    let exit = match &outcome {
        Ok(_) => Exit::Done,
        Err(ErrorBody::Kernel(error)) => kernel_exit(error),
        Err(ErrorBody::Client(error)) => error.code().exit(),
    };
    let envelope = Envelope {
        command: command.command.to_string(),
        instance,
        outcome,
    };
    let Err(EmitError {
        stream: Stream::Stdout,
        error,
    }) = emit(&envelope, form, stdout, stderr)
    else {
        return exit;
    };
    let failed = Envelope {
        outcome: Err(ErrorBody::Client(local_io(&error))),
        ..envelope
    };
    let _ = emit(&failed, form, stdout, stderr);
    Exit::Unavailable
}

fn request(command: &Command, name: &Option<String>) -> Result<String, usize> {
    let params = name
        .iter()
        .map(|name| ("name".to_string(), Json::String(name.clone())))
        .collect();
    let request = request_line(command.command, params)
        .expect("a request with at most one param repeats no key");
    let bytes = request.len() - 1;
    if bytes > MAX_REQUEST_BYTES {
        return Err(bytes);
    }
    Ok(request)
}

fn too_large(
    bytes: usize,
    named: Named,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let (source, argument) = if named.from_variable {
        (VARIABLE, VARIABLE.to_string())
    } else {
        ("--kernel", named.instance.name.clone().unwrap_or_default())
    };
    let error = client(
        ClientCode::InvalidArgument,
        format!(
            "{source}'s name makes the request line {bytes} bytes before its newline, and a \
             daemon reads at most {MAX_REQUEST_BYTES}"
        ),
        vec![("argument".to_string(), Json::String(argument))],
    );
    let exit = error.code().exit();
    let envelope = Envelope {
        command: INVALID.to_string(),
        instance: Some(named.instance),
        outcome: Err(ErrorBody::Client(error)),
    };
    let _ = emit(&envelope, form, stdout, stderr);
    exit
}

fn answer(
    command: &Command,
    stream: UnixStream,
    socket: &Path,
    request: &str,
    trusted_uid: u32,
    deadline: &Deadline<'_>,
) -> Result<Json, ErrorBody> {
    let line = exchange(stream, socket, trusted_uid, request, deadline).map_err(|unreplied| {
        ErrorBody::Client(match unreplied {
            Unreplied::NotSent(error) => error,
            Unreplied::Unanswered(failure) => unanswered(failure, command.changes_state),
            Unreplied::CutOff(error) => cut_off(&error, command.changes_state),
        })
    })?;
    match judge_reply(&line).map_err(ErrorBody::Client)? {
        Reply::Result(result) => Ok(result),
        Reply::Kernel(error) => Err(ErrorBody::Kernel(error)),
    }
}
