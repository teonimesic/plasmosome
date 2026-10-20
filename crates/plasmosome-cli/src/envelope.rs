use std::io::{self, Write};

use crate::codes::ClientCode;
use crate::json::{self, Json, Sink, Style};

const PIECE: usize = 64 * 1024;

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

/// Writes `envelope` in `form`, ending with a newline, then flushes. A success goes to `stdout`
/// and a failure to `stderr`; the other stream receives nothing. The text goes out in pieces of
/// about 64 KiB, so a large reply is never held twice.
pub fn emit(
    envelope: &Envelope,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), EmitError> {
    let (stream, written) = match envelope.outcome {
        Ok(_) => (Stream::Stdout, write_envelope(envelope, form, stdout)),
        Err(_) => (Stream::Stderr, write_envelope(envelope, form, stderr)),
    };
    written.map_err(|error| EmitError { stream, error })
}

fn write_envelope(envelope: &Envelope, form: Form, writer: &mut dyn Write) -> io::Result<()> {
    let mut out = Pieces {
        text: String::with_capacity(PIECE),
        writer,
        failed: None,
    };
    match (form, &envelope.outcome) {
        (Form::Json, _) => write_view(&mut out, &envelope.view(), Style::Compact, 0),
        (Form::Human, Ok(_)) => write_view(&mut out, &envelope.view(), Style::Human, 0),
        (Form::Human, Err(error)) => error.write_human(&mut out),
    }
    out.push('\n');
    out.finish()
}

struct Pieces<'a> {
    text: String,
    writer: &'a mut dyn Write,
    failed: Option<io::Error>,
}

impl Sink for Pieces<'_> {
    fn push_str(&mut self, text: &str) {
        self.text.push_str(text);
        if self.text.len() >= PIECE {
            self.spill();
        }
    }
}

impl Pieces<'_> {
    fn spill(&mut self) {
        if self.failed.is_none()
            && let Err(error) = self.writer.write_all(self.text.as_bytes())
        {
            self.failed = Some(error);
        }
        self.text.clear();
    }

    fn finish(mut self) -> io::Result<()> {
        self.spill();
        match self.failed.take() {
            Some(error) => Err(error),
            None => self.writer.flush(),
        }
    }
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

    fn write_human(&self, out: &mut impl Sink) {
        match self {
            ErrorBody::Client(error) => {
                json::show(out, &error.message);
                write_fix(out, &error.fields);
                out.push_str("\n(client ");
                out.push_str(error.code.as_str());
            }
            ErrorBody::Kernel(error) => {
                plain(out, json::member(&error.members, "message"));
                write_fix(out, &error.members);
                out.push_str("\n(kernel ");
                plain(out, json::member(&error.members, "code"));
            }
        }
        out.push(')');
    }
}

fn write_fix(out: &mut impl Sink, fields: &[(String, Json)]) {
    if let Some(fix) = json::member(fields, "fix") {
        out.push_str("\nfix: ");
        plain(out, Some(fix));
    }
}

fn plain(out: &mut impl Sink, value: Option<&Json>) {
    match value {
        None => {}
        Some(Json::String(text)) => json::show(out, text),
        Some(Json::Number(number)) => out.push_str(number.as_str()),
        Some(other) => json::write_value(out, other, Style::Human, 0),
    }
}

fn write_view(out: &mut impl Sink, view: &View<'_>, style: Style, depth: usize) {
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

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use super::{PIECE, Pieces};
    use crate::json::Sink;

    struct Refuses;

    impl Write for Refuses {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_failed_piece_is_not_kept() {
        let mut writer = Refuses;
        let mut out = Pieces {
            text: String::new(),
            writer: &mut writer,
            failed: None,
        };
        let line = "x".repeat(1024);
        for _ in 0..4 * PIECE / line.len() {
            out.push_str(&line);
            assert!(
                out.text.len() < PIECE,
                "{} bytes are held after the stream failed",
                out.text.len()
            );
        }
        let error = out.finish().expect_err("the stream refuses every write");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
