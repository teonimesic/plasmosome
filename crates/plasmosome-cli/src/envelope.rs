use std::io::{self, Write};

use crate::codes::ClientCode;
use crate::json::{self, Json, Style};

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
    code: ClientCode,
    message: String,
    fields: Vec<(String, Json)>,
}

impl ClientError {
    /// Takes the code, prose for a person, and the structured fields spec 025 names for `code` in
    /// the order to write them. Hands `fields` back when a key repeats or is `domain`, `code` or
    /// `message`, since writing it would repeat a key.
    pub fn new(
        code: ClientCode,
        message: String,
        fields: Vec<(String, Json)>,
    ) -> Result<ClientError, Vec<(String, Json)>> {
        let clashes = fields.iter().enumerate().any(|(index, (key, _))| {
            matches!(key.as_str(), "domain" | "code" | "message")
                || json::member(&fields[..index], key).is_some()
        });
        if clashes {
            return Err(fields);
        }
        Ok(ClientError {
            code,
            message,
            fields,
        })
    }

    /// Returns the client code.
    pub fn code(&self) -> ClientCode {
        self.code
    }

    /// Returns the message. A caller branches on the code and the fields, never on this.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the structured fields in the order they are written.
    pub fn fields(&self) -> &[(String, Json)] {
        &self.fields
    }
}

/// The daemon's error object, relayed unchanged with `domain: "kernel"` written first.
#[derive(Debug, PartialEq, Eq)]
pub struct KernelError {
    members: Vec<(String, Json)>,
}

impl KernelError {
    /// Takes the daemon's `error` value. Hands it back when it is not an object or already has a
    /// `domain` key, since relaying either would change what the daemon sent.
    pub fn new(mut error: Json) -> Result<KernelError, Json> {
        if let Json::Object(members) = &mut error
            && json::member(members, "domain").is_none()
        {
            return Ok(KernelError {
                members: std::mem::take(members),
            });
        }
        Err(error)
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
    let mut text = match (form, &envelope.outcome) {
        (Form::Json, _) => written(&envelope.view(), Style::Compact),
        (Form::Human, Ok(_)) => written(&envelope.view(), Style::Human),
        (Form::Human, Err(error)) => error.human(),
    };
    text.push('\n');
    let (stream, written) = match envelope.outcome {
        Ok(_) => (Stream::Stdout, write_line(stdout, &text)),
        Err(_) => (Stream::Stderr, write_line(stderr, &text)),
    };
    written.map_err(|error| EmitError { stream, error })
}

fn write_line(writer: &mut dyn Write, text: &str) -> io::Result<()> {
    writer.write_all(text.as_bytes())?;
    writer.flush()
}

enum View<'a> {
    Json(&'a Json),
    Text(&'a str),
    Raw(&'a str),
    Object(Vec<(&'a str, View<'a>)>),
}

impl Envelope {
    fn view(&self) -> View<'_> {
        let mut members = vec![
            ("schema", View::Raw("1")),
            ("command", View::Text(&self.command)),
        ];
        if let Some(instance) = &self.instance {
            let mut fields = Vec::new();
            if let Some(name) = &instance.name {
                fields.push(("name", View::Text(name)));
            }
            fields.push(("socket", View::Text(&instance.socket)));
            members.push(("instance", View::Object(fields)));
        }
        match &self.outcome {
            Ok(result) => members.push(("result", View::Json(result))),
            Err(error) => members.push(("error", error.view())),
        }
        View::Object(members)
    }
}

impl ErrorBody {
    fn view(&self) -> View<'_> {
        let (mut members, fields) = match self {
            ErrorBody::Client(error) => (
                vec![
                    ("domain", View::Text("client")),
                    ("code", View::Text(error.code.as_str())),
                    ("message", View::Text(&error.message)),
                ],
                &error.fields,
            ),
            ErrorBody::Kernel(error) => (vec![("domain", View::Text("kernel"))], &error.members),
        };
        members.extend(
            fields
                .iter()
                .map(|(key, value)| (key.as_str(), View::Json(value))),
        );
        View::Object(members)
    }

    fn human(&self) -> String {
        let (domain, code, message, fields) = match self {
            ErrorBody::Client(error) => (
                "client",
                error.code.as_str().to_string(),
                json::show(&error.message),
                &error.fields,
            ),
            ErrorBody::Kernel(error) => (
                "kernel",
                plain(json::member(&error.members, "code")),
                plain(json::member(&error.members, "message")),
                &error.members,
            ),
        };
        let mut text = message;
        if let Some(fix) = json::member(fields, "fix") {
            text.push_str("\nfix: ");
            text.push_str(&plain(Some(fix)));
        }
        text.push_str(&format!("\n({domain} {code})"));
        text
    }
}

fn plain(value: Option<&Json>) -> String {
    match value {
        None => String::new(),
        Some(Json::String(text)) => json::show(text),
        Some(Json::Number(number)) => number.as_str().to_string(),
        Some(other) => json::write_human(other),
    }
}

fn written(view: &View<'_>, style: Style) -> String {
    let mut out = String::new();
    write_view(&mut out, view, style, 0);
    out
}

fn write_view(out: &mut String, view: &View<'_>, style: Style, depth: usize) {
    match view {
        View::Json(value) => json::write_value(out, value, style, depth),
        View::Text(text) => json::write_string(out, text, style),
        View::Raw(text) => out.push_str(text),
        View::Object(members) => {
            out.push('{');
            for (index, (key, value)) in members.iter().enumerate() {
                json::separator(out, index == 0, style, depth + 1);
                json::write_key(out, key, style);
                write_view(out, value, style, depth + 1);
            }
            json::close_container(out, '}', style, depth);
        }
    }
}
