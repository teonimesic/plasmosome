use std::fmt;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

use super::{BootToken, cut};
use crate::universe::GrantId;
use crate::wire::{ObjectOnly, object_serde};

const REFUSED: i64 = 105;
const REFUSED_FROM: &str = "guest_request";
const REFUSED_TO: &str = "refused";

/// A request's `id`, kept as the exact JSON text the caller sent, so a reply
/// echoes it unchanged (spec 001 §1). Two ids are equal when their texts are
/// equal: `1` and `1.0` differ.
#[derive(Debug, Clone)]
pub struct RequestId(Box<RawValue>);

impl RequestId {
    /// The id `id`, written in decimal.
    pub fn number(id: u64) -> RequestId {
        RequestId(raw(id.to_string()))
    }

    /// The `null` id, under which a line whose own id cannot be trusted is
    /// answered.
    pub fn null() -> RequestId {
        RequestId(raw("null".to_owned()))
    }

    /// The id as an unsigned 64-bit integer, when its text is exactly that
    /// integer in decimal. `-0`, `1.0`, `1e0`, `"1"` and anything past
    /// `u64::MAX` give `None`.
    pub fn as_u64(&self) -> Option<u64> {
        self.0.get().parse().ok()
    }

    /// The id's JSON text, exactly as it was received or built.
    pub fn as_json(&self) -> &str {
        self.0.get()
    }
}

impl PartialEq for RequestId {
    fn eq(&self, other: &RequestId) -> bool {
        self.as_json() == other.as_json()
    }
}

impl Eq for RequestId {}

impl Serialize for RequestId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

fn raw(text: String) -> Box<RawValue> {
    RawValue::from_string(text).expect("a decimal number or null is one JSON value")
}

/// A protocol failure of spec 001 §1, which reuses the JSON-RPC reserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolCode {
    /// -32700: the line is not UTF-8 or not one JSON value.
    ParseError,
    /// -32600: the line is JSON but not a request this lane can read.
    InvalidRequest,
    /// -32601: no method of that name on this lane.
    MethodNotFound,
    /// -32602: the method's params do not decode.
    InvalidParams,
    /// -32603: the responder failed, not the request.
    InternalError,
}

impl ProtocolCode {
    /// The code's integer on the wire.
    pub fn code(self) -> i64 {
        match self {
            ProtocolCode::ParseError => -32700,
            ProtocolCode::InvalidRequest => -32600,
            ProtocolCode::MethodNotFound => -32601,
            ProtocolCode::InvalidParams => -32602,
            ProtocolCode::InternalError => -32603,
        }
    }
}

/// The `detail` of a 105 refusal a bridge responder sends.
///
/// In JSON it is an object whose `kind` names the variant, for example
/// `{"kind": "rejected", "reason": "..."}`. Decoding refuses an array, an
/// unknown kind, and a missing, repeated or unknown field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    remote = "Self",
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RefusalDetail {
    /// The guest refused hello, observe, install, activate, drain, remove or
    /// shutdown (spec 001 §4.2). `reason` is for people: a caller never
    /// branches on it.
    Rejected { reason: String },
}

object_serde!(RefusalDetail through RefusalDetail);

/// The error object of a failed bridge request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// A protocol failure, written as exactly `{code, message}`.
    Protocol { code: ProtocolCode, message: String },
    /// A refusal, written as exactly
    /// `{code: 105, message, from: "guest_request", to: "refused", detail}`.
    Refused {
        message: String,
        detail: RefusalDetail,
    },
}

/// A request a lane's decoder accepted: the id its reply must echo and the
/// typed call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundRequest<C> {
    pub id: RequestId,
    pub call: C,
}

/// A request line the responder refuses: answer it with
/// `encode_failure(&id, &error)` and read the next line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRefusal {
    /// The request's own id, or `null` when the line held no id that can be
    /// trusted.
    pub id: RequestId,
    pub error: BridgeError,
}

/// Why a request or success reply could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// Params encoded as something other than an object. `found` keeps at
    /// most 1,024 bytes of that JSON, ending on a character boundary.
    ParamsNotObject { found: String },
    /// The caller's serializer failed with this message.
    Serialize { message: String },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParamsNotObject { found } => {
                write!(
                    f,
                    "request params must encode as a JSON object, not {found}"
                )
            }
            Self::Serialize { message } => write!(f, "bridge value failed to encode: {message}"),
        }
    }
}

impl std::error::Error for EncodeError {}

#[derive(Serialize)]
struct RequestLine<'a> {
    id: u64,
    method: &'a str,
    params: &'a RawValue,
}

#[derive(Serialize)]
struct SuccessLine<'a> {
    id: &'a RequestId,
    result: &'a RawValue,
}

#[derive(Serialize)]
struct FailureLine<'a, E> {
    id: &'a RequestId,
    error: E,
}

#[derive(Serialize)]
struct ProtocolObject<'a> {
    code: i64,
    message: &'a str,
}

#[derive(Serialize)]
struct RefusalObject<'a> {
    code: i64,
    message: &'a str,
    from: &'static str,
    to: &'static str,
    detail: &'a RefusalDetail,
}

#[derive(Deserialize)]
#[serde(
    deny_unknown_fields,
    expecting = "a request object with exactly id, method and params"
)]
struct Envelope {
    id: Box<RawValue>,
    method: String,
    params: Box<RawValue>,
}

/// The compact request line `{"id":id,"method":method,"params":{...}}`, with
/// its keys in that order and no newline: the writer adds one.
///
/// # Errors
///
/// Returns [`EncodeError`] when `params` fails to encode or encodes as
/// anything but a JSON object. Formatting whitespace in raw JSON values is
/// removed at every depth; string contents are kept unchanged.
pub fn encode_request<P: Serialize>(
    id: u64,
    method: &str,
    params: &P,
) -> Result<Vec<u8>, EncodeError> {
    let params = compact(params)?;
    if !params.get().starts_with('{') {
        return Err(EncodeError::ParamsNotObject {
            found: cut(params.get()),
        });
    }
    Ok(encode(&RequestLine {
        id,
        method,
        params: &params,
    }))
}

/// The compact success reply `{"id":id,"result":...}`, echoing `id`'s text
/// unchanged, with no newline.
///
/// # Errors
///
/// Returns [`EncodeError::Serialize`] when `result` fails to encode.
/// Formatting whitespace in raw JSON values is removed at every depth;
/// string contents and the id's exact text are kept unchanged.
pub fn encode_success<R: Serialize>(id: &RequestId, result: &R) -> Result<Vec<u8>, EncodeError> {
    let result = compact(result)?;
    Ok(encode(&SuccessLine {
        id,
        result: &result,
    }))
}

/// The compact failure reply `{"id":id,"error":{...}}`, echoing `id`'s text
/// unchanged, with the error object exactly as [`BridgeError`] describes and
/// no newline. Its closed error types, strings and validated id cannot fail
/// to encode; no caller-provided serializer runs.
pub fn encode_failure(id: &RequestId, error: &BridgeError) -> Vec<u8> {
    match error {
        BridgeError::Protocol { code, message } => encode(&FailureLine {
            id,
            error: ProtocolObject {
                code: code.code(),
                message,
            },
        }),
        BridgeError::Refused { message, detail } => encode(&FailureLine {
            id,
            error: RefusalObject {
                code: REFUSED,
                message,
                from: REFUSED_FROM,
                to: REFUSED_TO,
                detail,
            },
        }),
    }
}

/// Splits one request line, passed without its newline, into its id, its
/// method name and its params object, still as JSON text.
///
/// A line that is not UTF-8 or not exactly one JSON value is -32700 under a
/// `null` id. A JSON value that is not an object with exactly `id`, `method`
/// and `params`, each once and `method` a string, is -32600 under a `null`
/// id, since its id cannot be trusted. Params that are not a JSON object are
/// -32600 under the request's id. The id may be any JSON value and is kept
/// as its exact text. A refusal's message keeps at most 1,024 bytes of the
/// decoder's text. The caller's reader bounds the line's length.
pub fn split_request(line: &[u8]) -> Result<(RequestId, String, Box<RawValue>), RequestRefusal> {
    let Ok(text) = std::str::from_utf8(line) else {
        return Err(refused(
            RequestId::null(),
            ProtocolCode::ParseError,
            "the request line is not UTF-8",
        ));
    };
    if let Err(error) = serde_json::from_str::<&RawValue>(text) {
        let message = format!("the request line is not one JSON value: {error}");
        return Err(refused(
            RequestId::null(),
            ProtocolCode::ParseError,
            &message,
        ));
    }
    let mut reader = serde_json::Deserializer::from_str(text);
    let envelope = Envelope::deserialize(ObjectOnly::new(&mut reader)).map_err(|error| {
        let message = format!("the request is not exactly {{id, method, params}}: {error}");
        refused(RequestId::null(), ProtocolCode::InvalidRequest, &message)
    })?;
    let id = RequestId(envelope.id);
    if !envelope.params.get().starts_with('{') {
        let message = "the request params are not a JSON object";
        return Err(refused(id, ProtocolCode::InvalidRequest, message));
    }
    Ok((id, envelope.method, envelope.params))
}

fn refused(id: RequestId, code: ProtocolCode, message: &str) -> RequestRefusal {
    RequestRefusal {
        id,
        error: BridgeError::Protocol {
            code,
            message: cut(message),
        },
    }
}

/// Why a reply cannot answer a control call. Guest text is kept at most 1,024 bytes,
/// ending on a character boundary. Callers match variants, never message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyFault {
    NotUtf8,
    NotJson {
        detail: String,
    },
    Envelope {
        detail: String,
    },
    WrongId {
        expected: u64,
        found: String,
    },
    Unattributed(BridgeError),
    Refused(BridgeError),
    Result {
        detail: String,
    },
    BootChanged {
        expected: BootToken,
        found: BootToken,
    },
    GrantMismatch {
        expected: GrantId,
        found: GrantId,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyEnvelope {
    id: Box<RawValue>,
    #[serde(default, deserialize_with = "present_raw")]
    result: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    error: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorObject {
    code: i64,
    message: String,
    #[serde(default, deserialize_with = "present_raw")]
    from: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    to: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_raw")]
    detail: Option<Box<RawValue>>,
}

fn present_raw<'de, D: serde::Deserializer<'de>>(
    reader: D,
) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(reader).map(Some)
}

fn decode_error(text: &str) -> Result<BridgeError, String> {
    let mut reader = serde_json::Deserializer::from_str(text);
    let error = ErrorObject::deserialize(ObjectOnly::new(&mut reader))
        .map_err(|error| error.to_string())?;
    let protocol = match error.code {
        -32700 => Some(ProtocolCode::ParseError),
        -32600 => Some(ProtocolCode::InvalidRequest),
        -32601 => Some(ProtocolCode::MethodNotFound),
        -32602 => Some(ProtocolCode::InvalidParams),
        -32603 => Some(ProtocolCode::InternalError),
        _ => None,
    };
    if let Some(code) = protocol
        && error.from.is_none()
        && error.to.is_none()
        && error.detail.is_none()
    {
        return Ok(BridgeError::Protocol {
            code,
            message: cut(&error.message),
        });
    }
    let string_is = |value: &Option<Box<RawValue>>, expected: &str| {
        value
            .as_ref()
            .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
            .is_some_and(|found| found == expected)
    };
    if error.code == REFUSED
        && string_is(&error.from, REFUSED_FROM)
        && string_is(&error.to, REFUSED_TO)
        && let Some(detail) = error.detail
    {
        let RefusalDetail::Rejected { reason } =
            serde_json::from_str(detail.get()).map_err(|error| error.to_string())?;
        return Ok(BridgeError::Refused {
            message: cut(&error.message),
            detail: RefusalDetail::Rejected {
                reason: cut(&reason),
            },
        });
    }
    Err("error fields do not match a protocol error or a guest refusal".to_owned())
}

/// Reads one reply without its newline, returning the result's original JSON text.
/// The reader enforces `MAX_FRAME_BYTES` and its deadline. JSON nesting is limited
/// to 128 levels. Exactly `{id, result}` or `{id, error}` is accepted; every field
/// occurs once. A null-id error is unattributed; other ids must be the expected u64.
pub fn split_reply(expected_id: u64, line: &[u8]) -> Result<Box<RawValue>, ReplyFault> {
    let text = std::str::from_utf8(line).map_err(|_| ReplyFault::NotUtf8)?;
    let not_json = |detail: &str| ReplyFault::NotJson {
        detail: cut(detail),
    };
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 128 {
                        return Err(not_json("recursion limit exceeded"));
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    serde_json::from_str::<&RawValue>(text).map_err(|error| not_json(&error.to_string()))?;
    let envelope_fault = |detail: &str| ReplyFault::Envelope {
        detail: cut(detail),
    };
    let mut reader = serde_json::Deserializer::from_str(text);
    let reply = ReplyEnvelope::deserialize(ObjectOnly::new(&mut reader))
        .map_err(|error| envelope_fault(&error.to_string()))?;
    if reply.result.is_some() == reply.error.is_some() {
        return Err(envelope_fault(
            "exactly one of result and error is required",
        ));
    }
    let error = reply
        .error
        .map(|error| decode_error(error.get()))
        .transpose()
        .map_err(|detail| envelope_fault(&detail))?;
    if reply.id.get() == "null"
        && let Some(error) = &error
    {
        return Err(ReplyFault::Unattributed(error.clone()));
    }
    if RequestId(reply.id.clone()).as_u64() != Some(expected_id) {
        return Err(ReplyFault::WrongId {
            expected: expected_id,
            found: cut(reply.id.get()),
        });
    }
    if let Some(error) = error {
        return Err(ReplyFault::Refused(error));
    }
    Ok(reply
        .result
        .expect("the envelope has exactly one result or error"))
}

fn encode(line: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(line).expect("a bridge line encodes as JSON")
}

fn compact(value: &impl Serialize) -> Result<Box<RawValue>, EncodeError> {
    let mut text = serde_json::to_string(value).map_err(|error| EncodeError::Serialize {
        message: error.to_string(),
    })?;
    let mut quoted = false;
    let mut escaped = false;
    text.retain(|ch| {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            true
        } else if ch == '"' {
            quoted = true;
            true
        } else {
            !matches!(ch, ' ' | '\t' | '\n' | '\r')
        }
    });
    Ok(RawValue::from_string(text).expect("removing JSON formatting keeps valid JSON"))
}
