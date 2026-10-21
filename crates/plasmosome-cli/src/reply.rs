use std::io::{self, BufRead, Read};

use crate::codes::{ClientCode, Exit};
use crate::envelope::{ClientError, KernelError};
use crate::json::{self, Json, MAX_REPLY_BYTES, Number};

const ID_IS_NOT_ONE: &str = "the reply's id is not 1";

/// Why [`read_reply`] returned no line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadFailure {
    /// The input ended, within the bound, before a newline. When more than [`MAX_REPLY_BYTES`]
    /// came before the end, the failure is [`ReadFailure::TooLarge`] instead.
    Closed,
    /// The line passed [`MAX_REPLY_BYTES`] before its newline.
    TooLarge,
    /// A read reported `WouldBlock` or `TimedOut`: the reader's timeout passed.
    TimedOut,
    /// A read failed for another reason. `errno` is the operating system's error number, when the
    /// error has one.
    Transport { errno: Option<i32> },
}

/// Reads one reply line from `reader` and returns it without its newline.
///
/// It consumes the line and its newline and nothing more, so whatever follows stays in `reader`
/// unread. It reads and keeps at most [`MAX_REPLY_BYTES`] plus one byte. When that last byte is
/// not the newline, it returns [`ReadFailure::TooLarge`] without reading further. An interrupted
/// read is retried.
///
/// It has no clock. Each read waits as long as `reader` lets it, so a caller with a deadline gives
/// each read only the time left, not one timeout for the whole line.
pub fn read_reply(reader: &mut dyn BufRead) -> Result<Vec<u8>, ReadFailure> {
    let mut line = Vec::new();
    let mut limited = reader.take(MAX_REPLY_BYTES as u64 + 1);
    if let Err(error) = limited.read_until(b'\n', &mut line) {
        return Err(failure_of(&error));
    }
    if line.pop_if(|byte| *byte == b'\n').is_some() {
        Ok(line)
    } else if line.len() > MAX_REPLY_BYTES {
        Err(ReadFailure::TooLarge)
    } else {
        Err(ReadFailure::Closed)
    }
}

fn failure_of(error: &io::Error) -> ReadFailure {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => ReadFailure::TimedOut,
        _ => ReadFailure::Transport {
            errno: error.raw_os_error(),
        },
    }
}

/// A reply that passed spec 025's rules.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// The daemon's `result` object, as it arrived.
    Result(Json),
    /// The daemon's `error` object, to relay with `domain: "kernel"` added.
    Kernel(KernelError),
}

/// Judges one reply line under spec 025 section 4, for a request sent with `id` 1.
///
/// A valid reply is an object with `id` 1 and exactly one of `result`, an object, and `error`, an
/// object with an integer `code`, a string `message` and no `domain`. A `null` id is valid only
/// for an error whose code is -32700 or -32600, which the daemon's connection loop sends. Other
/// members are ignored. Anything else is `bad_reply`, whose `detail` names the first rule broken.
pub fn judge_reply(line: &[u8]) -> Result<Reply, ClientError> {
    let mut reply = json::scan(line).map_err(|error| bad_reply(&error.to_string()))?;
    let Json::Object(members) = &mut reply else {
        return Err(bad_reply("the reply is not an object"));
    };
    let result = position(members, "result");
    let error = position(members, "error");
    match (result, error) {
        (Some(_), Some(_)) => Err(bad_reply("the reply has both result and error")),
        (None, None) => Err(bad_reply("the reply has neither result nor error")),
        (Some(index), None) => {
            if !is_one(json::member(members, "id")) {
                return Err(bad_reply(ID_IS_NOT_ONE));
            }
            match &mut members[index].1 {
                result @ Json::Object(_) => Ok(Reply::Result(std::mem::take(result))),
                _ => Err(bad_reply("the result is not an object")),
            }
        }
        (None, Some(index)) => {
            let error = std::mem::take(&mut members[index].1);
            judge_error(json::member(members, "id"), error)
        }
    }
}

fn judge_error(id: Option<&Json>, error: Json) -> Result<Reply, ClientError> {
    let code = code_of(error.get("code"));
    let from_the_connection_loop = matches!(code, Code::Integer(Some(-32700 | -32600)));
    if !(is_one(id) || (from_the_connection_loop && matches!(id, Some(Json::Null)))) {
        return Err(bad_reply(ID_IS_NOT_ONE));
    }
    if !matches!(error, Json::Object(_)) {
        return Err(bad_reply("the error is not an object"));
    }
    if code == Code::NotInteger {
        return Err(bad_reply("the error's code is not an integer"));
    }
    if !matches!(error.get("message"), Some(Json::String(_))) {
        return Err(bad_reply("the error's message is not a string"));
    }
    KernelError::new(error)
        .map(Reply::Kernel)
        .map_err(|_| bad_reply("the error already has a domain key"))
}

fn position(members: &[(String, Json)], key: &str) -> Option<usize> {
    members.iter().position(|(name, _)| name == key)
}

fn is_one(id: Option<&Json>) -> bool {
    matches!(id, Some(Json::Number(id)) if id.as_str() == "1")
}

#[derive(PartialEq, Eq)]
enum Code {
    NotInteger,
    Integer(Option<i64>),
}

fn code_of(code: Option<&Json>) -> Code {
    match code {
        Some(Json::Number(code)) if !code.as_str().contains(['.', 'e', 'E']) => {
            Code::Integer(code.as_str().parse().ok())
        }
        _ => Code::NotInteger,
    }
}

fn bad_reply(detail: &str) -> ClientError {
    ClientError::new(
        ClientCode::BadReply,
        "the daemon sent a reply this client cannot accept".to_string(),
        vec![("detail".to_string(), Json::String(detail.to_string()))],
    )
    .expect("a lone detail field neither repeats nor is reserved")
}

/// Returns the exit status spec 025 section 4 gives a kernel error, by its code.
///
/// 107 exits 1. 105 whose `to` is the string `"unknown"` exits 3. -32700, -32600 and -32603 exit
/// 70. Every other code exits 2, including one this client does not know, one outside `i64`, and
/// a missing code or one that is not an integer.
pub fn kernel_exit(error: &KernelError) -> Exit {
    let members = error.members();
    match code_of(json::member(members, "code")) {
        Code::Integer(Some(107)) => Exit::Unavailable,
        Code::Integer(Some(105)) if matches!(json::member(members, "to"), Some(Json::String(to)) if to == "unknown") => {
            Exit::OutcomeUnknown
        }
        Code::Integer(Some(-32700 | -32600 | -32603)) => Exit::Internal,
        _ => Exit::Refused,
    }
}

/// Returns the client error for a request whose reply line never arrived.
///
/// With `changes_state` false, the request cannot have changed anything. A closed connection, a
/// timeout or a failed read is `no_reply`, exit 1, with `cause` `closed`, `deadline` or
/// `transport`. A line past the bound is `reply_too_large`, exit 2, with `max_bytes`. With
/// `changes_state` true, the request may still take effect, so each is `outcome_unknown`, exit 3,
/// with `cause` `closed`, `deadline`, `transport` or `reply_too_large`.
pub fn unanswered(failure: ReadFailure, changes_state: bool) -> ClientError {
    let (cause, what) = match failure {
        ReadFailure::Closed => (
            "closed",
            "the daemon closed the connection before its reply was complete".to_string(),
        ),
        ReadFailure::TimedOut => (
            "deadline",
            "the deadline passed before the reply was complete".to_string(),
        ),
        ReadFailure::Transport { errno: Some(errno) } => (
            "transport",
            format!("reading the reply failed with errno {errno}"),
        ),
        ReadFailure::Transport { errno: None } => {
            ("transport", "reading the reply failed".to_string())
        }
        ReadFailure::TooLarge => (
            "reply_too_large",
            format!("the reply passed {MAX_REPLY_BYTES} bytes before its newline"),
        ),
    };
    let cause = Json::String(cause.to_string());
    let (code, message, field) = match (changes_state, failure) {
        (true, _) => (
            ClientCode::OutcomeUnknown,
            format!("{what}; the request may still take effect, so observe before repeating it"),
            ("cause", cause),
        ),
        (false, ReadFailure::TooLarge) => {
            (ClientCode::ReplyTooLarge, what, ("max_bytes", max_bytes()))
        }
        (false, _) => (
            ClientCode::NoReply,
            format!("{what}; nothing can have changed"),
            ("cause", cause),
        ),
    };
    ClientError::new(code, message, vec![(field.0.to_string(), field.1)])
        .expect("a lone field named cause or max_bytes neither repeats nor is reserved")
}

fn max_bytes() -> Json {
    Json::Number(
        Number::new(MAX_REPLY_BYTES.to_string()).expect("a decimal integer is a JSON number"),
    )
}

/// Builds the request line for `method` with `params`: `{"id":1,"method":...,"params":{...}}`,
/// written by [`json::write_compact`] and ended with a newline.
///
/// Hands `params` back when a key repeats among them or at any depth of a value, since the line
/// would then repeat a key.
pub fn request_line(
    method: &str,
    params: Vec<(String, Json)>,
) -> Result<String, Vec<(String, Json)>> {
    let repeats = params.iter().enumerate().any(|(index, (key, value))| {
        json::member(&params[..index], key).is_some() || json::repeats_a_key(value)
    });
    if repeats {
        return Err(params);
    }
    let method = json::write_compact(&Json::String(method.to_string()));
    let params = json::write_compact(&Json::Object(params));
    Ok(format!(
        "{{\"id\":1,\"method\":{method},\"params\":{params}}}\n"
    ))
}
