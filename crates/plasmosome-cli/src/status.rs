use std::io::{ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use plasmosome_core::private_socket::check_private_path;

use crate::cli::{Context, INVALID, Invocation, client, local_io, refuse};
use crate::clock::Deadline;
use crate::codes::{ClientCode, Exit};
use crate::commands::Command;
use crate::envelope::{ClientError, EmitError, Envelope, ErrorBody, Instance, Stream, emit};
use crate::exchange::{Unreplied, exchange};
use crate::json::Json;
use crate::refusal::{PathOrigin, at, map_refusal, number, refusal};
use crate::reply::{Reply, judge_reply, kernel_exit, request_line, unanswered};

#[cfg(target_os = "macos")]
const MAX_SOCKET_PATH: usize = 103;
#[cfg(not(target_os = "macos"))]
const MAX_SOCKET_PATH: usize = 107;

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
    } = invocation;
    let Some(socket) = socket else {
        let error = client(
            ClientCode::InvalidArgument,
            "plasmosome status needs --socket PATH in this build".to_string(),
            Vec::new(),
        );
        return refuse(INVALID.to_string(), error, form, stdout, stderr);
    };
    let params = kernel
        .iter()
        .map(|name| ("name".to_string(), Json::String(name.clone())))
        .collect();
    let request = request_line(command.command, params)
        .expect("a request with at most one param repeats no key");
    let bytes = request.len() - 1;
    if let Some(name) = &kernel
        && bytes > MAX_REQUEST_BYTES
    {
        let error = client(
            ClientCode::InvalidArgument,
            format!(
                "--kernel's name makes the request line {bytes} bytes before its newline, \
                 and a daemon reads at most {MAX_REQUEST_BYTES}"
            ),
            vec![("argument".to_string(), Json::String(name.clone()))],
        );
        return refuse(INVALID.to_string(), error, form, stdout, stderr);
    }
    let deadline = Deadline::new(
        context.clock,
        start,
        deadline_ms.unwrap_or(DEFAULT_DEADLINE_MS),
    );
    let outcome = answer(
        &command,
        &request,
        Path::new(&socket),
        context.trusted_uid,
        &deadline,
    );
    let exit = match &outcome {
        Ok(_) => Exit::Done,
        Err(ErrorBody::Kernel(error)) => kernel_exit(error),
        Err(ErrorBody::Client(error)) => error.code().exit(),
    };
    let envelope = Envelope {
        command: command.command.to_string(),
        instance: Some(Instance {
            name: kernel,
            socket,
        }),
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

fn answer(
    command: &Command,
    request: &str,
    socket: &Path,
    trusted_uid: u32,
    deadline: &Deadline<'_>,
) -> Result<Json, ErrorBody> {
    let stream = connect(socket, trusted_uid).map_err(ErrorBody::Client)?;
    let line = exchange(stream, socket, trusted_uid, request, deadline).map_err(|unreplied| {
        ErrorBody::Client(match unreplied {
            Unreplied::NotSent(error) => error,
            Unreplied::Unanswered(failure) => unanswered(failure, command.changes_state),
        })
    })?;
    match judge_reply(&line).map_err(ErrorBody::Client)? {
        Reply::Result(result) => Ok(result),
        Reply::Kernel(error) => Err(ErrorBody::Kernel(error)),
    }
}

fn connect(socket: &Path, trusted_uid: u32) -> Result<UnixStream, ClientError> {
    let length = socket.as_os_str().len();
    if length > MAX_SOCKET_PATH {
        return Err(refusal(
            ClientCode::SocketPathTooLong,
            format!(
                "{} is {length} bytes, longer than the {MAX_SOCKET_PATH} a socket address holds",
                socket.display()
            ),
            vec![at(socket), ("max_bytes", number(MAX_SOCKET_PATH))],
        ));
    }
    check_private_path(socket, trusted_uid)
        .map_err(|error| map_refusal(&error, socket, PathOrigin::SocketFlag, trusted_uid))?;
    UnixStream::connect(socket).map_err(|error| {
        let (code, what) = match error.kind() {
            ErrorKind::ConnectionRefused => (
                ClientCode::Unreachable,
                "refused the connection: its daemon may be stopped, or alive with a full backlog"
                    .to_string(),
            ),
            ErrorKind::NotFound => (
                ClientCode::NotRunning,
                "does not exist: no socket is bound at that name".to_string(),
            ),
            _ => (
                ClientCode::Transport,
                format!("could not be reached: {error}"),
            ),
        };
        let mut fields = vec![at(socket)];
        if code == ClientCode::Transport {
            fields.extend(error.raw_os_error().map(|errno| ("errno", number(errno))));
        }
        refusal(code, format!("{} {what}", socket.display()), fields)
    })
}
