use std::io::{self, Write};

use crate::codes::ClientCode;
use crate::json::Json;

/// What one command prints: spec 025's success or failure envelope.
#[derive(Debug, PartialEq, Eq)]
pub struct Envelope {
    /// The wire method, or `invalid` for a usage error.
    pub command: String,
    /// The socket the client selected. `None` when it had determined no socket path.
    pub instance: Option<Instance>,
    /// The daemon's `result` object, or the error that ended the command.
    pub outcome: Result<Json, ErrorBody>,
}

/// The selected socket, and the instance's name when the client knew one.
#[derive(Debug, PartialEq, Eq)]
pub struct Instance {
    pub name: Option<String>,
    pub socket: String,
}

/// The error object a failure envelope carries.
#[derive(Debug, PartialEq, Eq)]
pub enum ErrorBody {
    Client(ClientError),
    Kernel(KernelError),
}

/// The client's own refusal, written as `{"domain":"client","code":...,"message":...}` followed
/// by `fields`.
#[derive(Debug, PartialEq, Eq)]
pub struct ClientError {
    pub code: ClientCode,
    /// Prose for a person. A caller branches on `code` and `fields`, never on this.
    pub message: String,
    /// The structured fields spec 025 names for `code`, in the order to write them. A key must
    /// not repeat, and must not be `domain`, `code` or `message`.
    pub fields: Vec<(String, Json)>,
}

/// The daemon's error object, relayed unchanged with `domain: "kernel"` written first.
#[derive(Debug, PartialEq, Eq)]
pub struct KernelError {
    members: Vec<(String, Json)>,
}

impl KernelError {
    /// Takes the daemon's `error` value. Hands it back when it is not an object or already has a
    /// `domain` key, since relaying either would change what the daemon sent.
    pub fn new(error: Json) -> Result<KernelError, Json> {
        let _ = error;
        todo!()
    }

    /// Returns the daemon's members in the order they arrived.
    pub fn members(&self) -> &[(String, Json)] {
        &self.members
    }
}

/// Which form [`emit`] writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// One compact JSON line: the contract.
    Json,
    /// Output for a person, which nothing may parse.
    Human,
}

/// The stream a write failed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// A failed write. The caller decides the exit, since spec 025's rule depends on whether the
/// request changes state.
#[derive(Debug)]
pub struct EmitError {
    pub stream: Stream,
    pub error: io::Error,
}

/// Writes `envelope` in `form`, ending with a newline. A success goes to `stdout` and a failure
/// to `stderr`; the other stream receives nothing.
pub fn emit(
    envelope: &Envelope,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), EmitError> {
    let _ = (envelope, form, stdout, stderr);
    todo!()
}
